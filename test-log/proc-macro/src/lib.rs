// Copyright (C) 2019-2021 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: (Apache-2.0 OR MIT)

#![deny(rustdoc::broken_intra_doc_links, missing_docs)]

//! A crate providing a replacement #[[macro@test]] attribute that
//! initializes logging and/or tracing infrastructure before running
//! tests.

use darling::ast::NestedMeta;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, parse_quote, ItemFn, Meta, ReturnType};

/// A procedural macro for the `test` attribute.
///
/// The attribute can be used to define a test that has the `env_logger`
/// and/or `tracing` crates initialized (depending on the features used).
///
/// # Example
///
/// Specify the attribute on a per-test basis:
/// ```rust
/// # // doctests seemingly run in a slightly different environment where
/// # // `super`, which is what our macro makes use of, is not available.
/// # // By having a fake module here we work around that problem.
/// # #[cfg(feature = "log")]
/// # mod fordoctest {
/// # use logging::info;
/// # // Note that no test would actually run, regardless of `no_run`,
/// # // because we do not invoke the function.
/// #[nimiq_test_log::test]
/// fn it_works() {
///   info!("Checking whether it still works...");
///   assert_eq!(2 + 2, 4);
///   info!("Looks good!");
/// }
/// # }
/// ```
///
/// It can be very convenient to convert over all tests by overriding
/// the `#[test]` attribute on a per-module basis:
/// ```rust,no_run
/// # mod fordoctest {
/// use nimiq_test_log::test;
///
/// #[test]
/// fn it_still_works() {
///   // ...
/// }
/// # }
/// ```
///
/// You can also wrap another attribute. For example, suppose you use
/// [`#[test(tokio::test)]`](https://docs.rs/tokio/1.4.0/tokio/attr.test.html)
/// to run async tests:
/// ```
/// # mod fordoctest {
/// use nimiq_test_log::test;
///
/// #[test(tokio::test)]
/// async fn it_still_works() {
///   // ...
/// }
/// # }
/// ```
#[proc_macro_attribute]
pub fn test(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match NestedMeta::parse_meta_list(attr.into()) {
        Ok(v) => v,
        Err(_) => {
            panic!("unsupported arguments supplied: {}", quote! { attr });
        }
    };
    let input = parse_macro_input!(item as ItemFn);

    let inner_test = match args.as_slice() {
        [] => NestedMeta::Meta(Meta::Path(parse_quote! { ::core::prelude::v1::test })),
        [m] => m.clone(),
        _ => panic!("unsupported attributes supplied: {}", quote! { args }),
    };

    expand_wrapper(&inner_test, &input)
}

/// Emit code for a wrapper function around a test function.
fn expand_wrapper(inner_test: &NestedMeta, wrappee: &ItemFn) -> TokenStream {
    let attrs = &wrappee.attrs;
    let async_ = &wrappee.sig.asyncness;
    let await_ = if async_.is_some() {
        quote! {.await}
    } else {
        quote! {}
    };
    let body = &wrappee.block;
    let test_name = &wrappee.sig.ident;

    // Note that Rust does not allow us to have a test function with
    // #[should_panic] that has a non-unit return value.
    let ret = match &wrappee.sig.output {
        ReturnType::Default => quote! {},
        ReturnType::Type(_, type_) => quote! {-> #type_},
    };

    let result = quote! {
      #[#inner_test]
      #(#attrs)*
      #async_ fn #test_name() #ret {
        #async_ fn test_impl() #ret {
          #body
        }

        ::nimiq_test_log::initialize();

        test_impl()#await_
      }
    };
    result.into()
}
