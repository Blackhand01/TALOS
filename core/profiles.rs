#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionProfile {
    Sitl,
    Hitl,
}

impl ExecutionProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sitl" => Some(Self::Sitl),
            "hitl" => Some(Self::Hitl),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Sitl => "sitl",
            Self::Hitl => "hitl",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_execution_profiles() {
        assert_eq!(
            ExecutionProfile::parse("sitl"),
            Some(ExecutionProfile::Sitl)
        );
        assert_eq!(
            ExecutionProfile::parse("hitl"),
            Some(ExecutionProfile::Hitl)
        );
        assert_eq!(ExecutionProfile::parse("mixed"), None);
    }
}
