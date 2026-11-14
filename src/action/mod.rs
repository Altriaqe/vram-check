//! 行动层：只做"写"操作，所有风险判断在调用前完成。

pub mod close;

pub use close::{is_alive, post_close, terminate};
