#![allow(unused_imports)]

pub use self::inner::*;

#[macro_use]
#[cfg(feature = "cost-analysis")]
pub mod inner {
    pub use colored::Colorize;
    use std::sync::atomic::AtomicUsize;
    pub static NUM_INDENT: AtomicUsize = AtomicUsize::new(0);
    pub const PAD_CHAR: &'static str = "·";

    pub struct CostInfo {
        pub msg: String,
        pub num_constraints: usize,
    }

    #[macro_export]
    macro_rules! start_cost_analysis {
        ($cs:expr, $msg:expr) => {{
            use std::sync::atomic::Ordering;
            use $crate::cost_analysis::inner::{compute_indent, Colorize, NUM_INDENT};

            let msg = $msg();
            let start_info = "Start:".yellow().bold();
            let indent_amount = 2 * NUM_INDENT.fetch_add(0, Ordering::Relaxed);
            let indent = compute_indent(indent_amount);

            println!("{}{:8} {}", indent, start_info, msg);
            NUM_INDENT.fetch_add(1, Ordering::Relaxed);
            $crate::cost_analysis::inner::CostInfo {
                msg: msg.to_string(),
                num_constraints: $cs.num_constraints(),
            }
        }};
    }

    #[macro_export]
    macro_rules! end_cost_analysis {
        ($cs:expr, $cost:expr) => {{
            end_cost_analysis!($cs, $cost, || "");
        }};
        ($cs:expr, $cost:expr, $msg:expr) => {{
            use std::sync::atomic::Ordering;
            use $crate::cost_analysis::inner::{compute_indent, Colorize, NUM_INDENT};

            let num_constraints = $cs.num_constraints();
            let final_constraints = num_constraints - $cost.num_constraints;

            let end_info = "End:".green().bold();
            let message = format!("{} {}", $cost.msg, $msg());

            NUM_INDENT.fetch_sub(1, Ordering::Relaxed);
            let indent_amount = 2 * NUM_INDENT.fetch_add(0, Ordering::Relaxed);
            let indent = compute_indent(indent_amount);

            println!(
                "{}{:8} {:.<pad$}{}",
                indent,
                end_info,
                message,
                final_constraints,
                pad = 75 - indent_amount
            );
        }};
    }

    #[macro_export]
    macro_rules! next_cost_analysis {
        ($cs:expr, $cost:expr) => {{
            next_cost_analysis!($cs, $cost, || "");
        }};
        ($cs:expr, $cost:expr, $msg:expr) => {{
            end_cost_analysis!($cs, $cost, || "");
            $cost = start_cost_analysis!($cs, $msg);
        }};
    }

    pub fn compute_indent(indent_amount: usize) -> String {
        let mut indent = String::new();
        for _ in 0..indent_amount {
            indent.push_str(&PAD_CHAR.white());
        }
        indent
    }
}

#[macro_use]
#[cfg(not(feature = "cost-analysis"))]
pub mod inner {
    pub struct CostInfo;

    #[macro_export]
    macro_rules! start_cost_analysis {
        ($cs:expr, $msg:expr) => {{
            let _ = $msg;
            $crate::cost_analysis::inner::CostInfo
        }};
    }

    #[macro_export]
    macro_rules! next_cost_analysis {
        ($cs:expr, $cost:expr) => {};
        ($cs:expr, $cost:expr, $msg:expr) => {{
            let _ = $msg;
        }};
    }

    #[macro_export]
    macro_rules! end_cost_analysis {
        ($cs:expr, $cost:expr) => {{
            let _ = $cost;
        }};
        ($cs:expr, $cost:expr, $msg:expr) => {{
            let _ = $msg;
            let _ = $cost;
        }};
    }
}
