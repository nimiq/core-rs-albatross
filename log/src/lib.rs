use std::{env, fmt};

use ansiterm::{Color, Style};
use log::{level_filters::LevelFilter, Event, Level, Subscriber};
use time::format_description::well_known::Iso8601;
use tracing_log::NormalizeEvent;
use tracing_subscriber::{
    filter::Targets,
    fmt::{
        format::Writer,
        time::{FormatTime, UtcTime},
        FmtContext, FormatEvent, FormatFields, FormattedFields,
    },
    registry::LookupSpan,
};

pub static NIMIQ_MODULES: &[&str] = &[
    "nimiq_account",
    "nimiq_block",
    "nimiq_blockchain",
    "nimiq_blokchain_interface",
    "nimiq_blockchain_proxy",
    "nimiq_bls",
    "nimiq_client",
    "nimiq_collections",
    "nimiq_consensus",
    "nimiq_database",
    "nimiq_genesis",
    "nimiq_genesis_builder",
    "nimiq_handel",
    "nimiq_hash",
    "nimiq_hash_derive",
    "nimiq_key_derivation",
    "nimiq_keys",
    "nimiq_lib",
    "nimiq_light_blockchain",
    "nimiq_log",
    "nimiq_macros",
    "nimiq_mempool",
    "nimiq_metrics_server",
    "nimiq_mmr",
    "nimiq_mnemonic",
    "nimiq_network_interface",
    "nimiq_network_libp2p",
    "nimiq_network_mock",
    "nimiq_pedersen_generators",
    "nimiq_verifying_keys",
    "nimiq_pow_migration",
    "nimiq_primitives",
    "nimiq_rpc",
    "nimiq_rpc_client",
    "nimiq_rpc_interface",
    "nimiq_rpc_server",
    "nimiq_serde",
    "nimiq_spammer",
    "nimiq_subscription",
    "nimiq_tendermint",
    "nimiq_test_utils",
    "nimiq_transaction",
    "nimiq_transaction_builder",
    "nimiq_trie",
    "nimiq_utils",
    "nimiq_validator",
    "nimiq_validator_network",
    "nimiq_vrf",
    "nimiq_wallet",
    "nimiq_web_client",
    "nimiq_zkp",
    "nimiq_zkp_circuits",
    "nimiq_zkp_component",
    "nimiq_zkp_primitives",
];

pub const ENV: &str = "RUST_LOG";

pub trait TargetsExt {
    fn with_nimiq_targets(self, level: LevelFilter) -> Self;
    fn with_env(self) -> Self;
}

impl TargetsExt for Targets {
    fn with_nimiq_targets(mut self, level: LevelFilter) -> Targets {
        for &module in NIMIQ_MODULES.iter() {
            self = self.with_target(module, level);
        }
        self
    }
    fn with_env(mut self) -> Targets {
        let directives = match env::var(ENV) {
            Ok(v) => v,
            Err(env::VarError::NotPresent) => return self,
            Err(env::VarError::NotUnicode(_)) => panic!("env var {ENV} contains non-UTF-8 value"),
        };
        for dir in directives.split(',') {
            let mut iter = dir.splitn(2, '=');
            let (target, level) = match (iter.next(), iter.next()) {
                (Some(first), Some(second)) => (Some(first), second),
                (Some(first), None) => (None, first),
                (_, _) => unreachable!(),
            };
            let level: LevelFilter = match level.parse() {
                Ok(l) => l,
                // Ignore invalid directives.
                Err(_) => continue,
            };
            if let Some(t) = target {
                if t == "nimiq" {
                    self = self.with_nimiq_targets(level);
                } else {
                    self = self.with_target(t, level);
                }
            } else {
                self = self.with_default(level);
            }
        }
        self
    }
}

pub struct Formatting<T: FormatTime>(pub T);

impl<S, N, T: FormatTime> FormatEvent<S, N> for Formatting<T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let (bold, dim) = if writer.has_ansi_escapes() {
            (Style::default().bold(), Style::default().dimmed())
        } else {
            (Style::default(), Style::default())
        };
        write!(&mut writer, "{}", dim.prefix())?;
        self.0.format_time(&mut writer)?;
        write!(&mut writer, "{}", dim.suffix())?;

        // Format values from the event's metadata:
        let normalized_metadata = event.normalized_metadata();
        let metadata = normalized_metadata
            .as_ref()
            .unwrap_or_else(|| event.metadata());

        let color = if writer.has_ansi_escapes() {
            Style::from(match *metadata.level() {
                Level::TRACE => Color::Purple,
                Level::DEBUG => Color::Blue,
                Level::INFO => Color::Green,
                Level::WARN => Color::Yellow,
                Level::ERROR => Color::Red,
            })
        } else {
            Style::default()
        };

        let mut target = metadata.target();
        // Drop everything except the module name.
        if let Some(pos) = target.rfind("::") {
            target = &target[pos + 2..];
        }
        // Pretend `target` is ASCII-only
        const MAX_MODULE_WIDTH: usize = 20;
        let truncate = target.len() > MAX_MODULE_WIDTH;
        if truncate {
            for i in (0..MAX_MODULE_WIDTH).rev() {
                if target.is_char_boundary(i) {
                    target = &target[..i];
                    break;
                }
            }
        }
        let indicator = if truncate {
            "…"
        } else if target.len() < MAX_MODULE_WIDTH {
            " "
        } else {
            ""
        };

        write!(
            &mut writer,
            " {}{:5}{} {}{:width$}{}{} | ",
            color.prefix(),
            metadata.level(),
            color.suffix(),
            dim.prefix(),
            target,
            indicator,
            dim.suffix(),
            width = MAX_MODULE_WIDTH - 1,
        )?;

        // Write fields on the event
        ctx.field_format().format_fields(writer.by_ref(), event)?;

        // Format all the spans in the event's span context.
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                write!(
                    writer,
                    ", {}{}{{{}",
                    bold.prefix(),
                    span.name(),
                    bold.suffix()
                )?;

                // `FormattedFields` is a formatted representation of the span's
                // fields, which is stored in its extensions by the `fmt` layer's
                // `new_span` method. The fields will have been formatted
                // by the same field formatter that's provided to the event
                // formatter in the `FmtContext`.
                let ext = span.extensions();
                let fields = &ext
                    .get::<FormattedFields<N>>()
                    .expect("will never be `None`");

                // Skip formatting the fields if the span had no fields.
                if !fields.is_empty() {
                    write!(writer, "{}{}}}{}", fields, bold.prefix(), bold.suffix())?;
                }
            }
        }

        writeln!(writer)?;

        Ok(())
    }
}

pub struct MaybeSystemTime(pub bool);

impl FormatTime for MaybeSystemTime {
    fn format_time(&self, w: &mut Writer) -> fmt::Result {
        if self.0 {
            UtcTime::new(Iso8601::DEFAULT).format_time(w)
        } else {
            ().format_time(w)
        }
    }
}
