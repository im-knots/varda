//! Turns an unresolvable UUID into a "not found" command result. Each owner
//! resolves its own entities (`Mixer::resolve_deck`, `Outputs::resolve_output`, ...).

pub use crate::engine::value::entity::UnknownEntity;

impl From<UnknownEntity> for crate::engine::CommandResult {
    fn from(e: UnknownEntity) -> Self {
        crate::engine::CommandResult::Err {
            code: crate::engine::ErrorCode::NotFound,
            message: e.to_string(),
        }
    }
}

impl From<crate::mixer::ArrangementError> for crate::engine::CommandResult {
    fn from(e: crate::mixer::ArrangementError) -> Self {
        let code = if e.is_not_found() {
            crate::engine::ErrorCode::NotFound
        } else {
            crate::engine::ErrorCode::InvalidInput
        };
        crate::engine::CommandResult::Err {
            code,
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{CommandResult, ErrorCode};

    #[test]
    fn into_command_result_is_not_found_with_display_message() {
        // Every resolve_* error path uses this `.into()` to build a 404.
        let e = UnknownEntity::new("effect", "fx-99");
        let expected = e.to_string();
        let result: CommandResult = e.into();
        match result {
            CommandResult::Err { code, message } => {
                assert_eq!(code, ErrorCode::NotFound);
                assert_eq!(message, expected);
            }
            other => panic!("expected Err(NotFound), got {other:?}"),
        }
    }
}
