//! Graphics pipelines for the wgpu backend.
//!
//! Not yet implemented. The error says so rather than building something
//! plausible, because a pipeline that creates and draws nothing is the single
//! most expensive bug to find later.

/// Build a render pipeline from an RHI description.
pub fn create(
    _resources: &mut crate::wgpu_resources::WgpuResources,
    desc: &chassis::PipelineDesc,
) -> chassis::Result<chassis::Pipeline> {
    Err(chassis::Error::Unsupported(format!(
        "the wgpu pipeline path is not written yet; wanted {:?}",
        desc.label
    )))
}
