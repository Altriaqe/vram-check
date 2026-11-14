//! 数据层：唯一的"读"操作。UI 不知道有 PDH，这里也不知道有窗口。

pub mod model;
pub mod sampler;

/// 采样失败的分类。区分"系统不支持"（要降级）和"这次失败"（要跳过）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleError {
    /// 计数器组打不开：多半是系统不支持（Win7 / 无 WDDM 计数器）。
    Open(u32),
    /// 本次采集失败：可以下一轮重试。
    Collect(u32),
    /// 读取计数器数组失败。
    Read(u32),
    /// 进程快照失败。
    ProcessSnapshot,
    /// 没有找到带显存的适配器。
    NoAdapter,
    /// 采样线程起不来。
    Thread,
}

impl SampleError {
    pub fn code(&self) -> usize {
        match self {
            SampleError::Open(code) | SampleError::Collect(code) | SampleError::Read(code) => {
                *code as usize
            }
            SampleError::ProcessSnapshot => 1,
            SampleError::NoAdapter => 3,
            SampleError::Thread => 2,
        }
    }

    /// 给用户看的完整句子；绝不出现错误码。
    pub fn user_text(&self) -> &'static str {
        match self {
            SampleError::Open(_) => "这台机器读不到显存计数器，无法显示显存数据。",
            SampleError::Collect(_) => "这次刷新失败了，下一轮会自动重试。",
            SampleError::Read(_) => "有一条显存数据读不出来，已跳过它而不是当作 0。",
            SampleError::ProcessSnapshot => "读不到进程列表，请稍后刷新。",
            SampleError::NoAdapter => "没有找到带显存的显卡，无法显示显存数据。",
            SampleError::Thread => "采样线程没能启动。",
        }
    }
}
