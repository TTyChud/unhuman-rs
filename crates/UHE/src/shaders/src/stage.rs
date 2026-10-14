//! Shader stages. One type for the whole crate so a stage cannot be a string.

use std::fmt;
use std::str::FromStr;

/// What a shader module is compiled for.
///
/// Not an arbitrary string: `slangc` silently compiles for the wrong stage if
/// the name is a typo, and the failure surfaces much later as a pipeline
/// creation error that blames the backend.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Stage {
    Vertex,
    Fragment,
    Compute,
    Geometry,
    Hull,
    Domain,
    Task,
    Mesh,
    Raygen,
    Miss,
    Callable,
    Intersection,
    Amplification,
}

impl Stage {
    /// The spelling `slangc -stage` wants.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Vertex => "vertex",
            Stage::Fragment => "fragment",
            Stage::Compute => "compute",
            Stage::Geometry => "geometry",
            Stage::Hull => "hull",
            Stage::Domain => "domain",
            Stage::Task => "task",
            Stage::Mesh => "mesh",
            Stage::Raygen => "raygen",
            Stage::Miss => "miss",
            Stage::Callable => "callable",
            Stage::Intersection => "intersection",
            Stage::Amplification => "amplification",
        }
    }

    /// True for stages that exist as a graphics or compute pipeline.
    ///
    /// `VulkanGraphicPipeline` and `VulkanComputePipeline` accept only these,
    /// and they do not infer: a task or mesh stage is a pipeline of its own and
    /// silently treating it as a vertex shader would produce a pipeline that
    /// creates successfully and draws nothing.
    pub fn is_pipeline_stage(self) -> bool {
        matches!(
            self,
            Stage::Vertex
                | Stage::Fragment
                | Stage::Compute
                | Stage::Geometry
                | Stage::Hull
                | Stage::Domain
                | Stage::Task
                | Stage::Mesh
                | Stage::Amplification
        )
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownStage(pub String);

impl FromStr for Stage {
    type Err = UnknownStage;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let stage = match value {
            "vertex" => Stage::Vertex,
            "fragment" | "pixel" => Stage::Fragment,
            "compute" => Stage::Compute,
            "geometry" => Stage::Geometry,
            "hull" => Stage::Hull,
            "domain" => Stage::Domain,
            "task" => Stage::Task,
            "mesh" => Stage::Mesh,
            "raygen" => Stage::Raygen,
            "miss" => Stage::Miss,
            "callable" => Stage::Callable,
            "intersection" => Stage::Intersection,
            "amplification" => Stage::Amplification,
            other => return Err(UnknownStage(other.to_owned())),
        };
        Ok(stage)
    }
}
