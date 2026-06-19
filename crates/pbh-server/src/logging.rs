//! tracing 日志装配：控制台 + 文件（按日滚动）+ 进程内环形缓冲（供 WS）。

use std::path::Path;
use std::sync::Arc;

use pbh_domain::LogBuffer;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

/// 把日志事件写入环形缓冲的 Layer。
struct BufferLayer {
    buf: Arc<LogBuffer>,
}

struct MsgVisitor {
    message: String,
}

impl Visit for MsgVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            use std::fmt::Write;
            let _ = write!(self.message, "{value:?}");
        }
    }
}

impl<S> Layer<S> for BufferLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = MsgVisitor {
            message: String::new(),
        };
        event.record(&mut v);
        let meta = event.metadata();
        self.buf
            .push(meta.level().as_str(), meta.target(), v.message);
    }
}

/// 初始化全局 tracing 订阅器。返回的 guard 必须在程序生命周期内保持存活（文件写线程）。
pub fn init(buf: Arc<LogBuffer>, logs_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=warn"));

    let file_appender = tracing_appender::rolling::daily(logs_dir, "pbh.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let console = tracing_subscriber::fmt::layer().with_target(false);
    let file = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(filter)
        .with(console)
        .with(file)
        .with(BufferLayer { buf })
        .init();

    guard
}
