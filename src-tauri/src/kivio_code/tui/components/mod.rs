//! 基础组件 —— `Text` / `BoxView` / `Spacer` / `TruncatedText`，均实现
//! [`Component`](super::render::Component)，emit padding 后的 ANSI 行字符串供差分渲染器 diff。

pub mod text;

#[allow(unused_imports)]
pub use text::{BoxView, Spacer, Text, TruncatedText};

/// 背景色函数：接收内容字符串，返回包了背景 SGR 的字符串。对应 PI 的 `(s)=>string`。
pub type ColorFn = std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>;
