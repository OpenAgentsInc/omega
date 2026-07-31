//! SCV: Space Construction Vehicle — hyper-lightweight read-only ACP agent (v0.1).

#![forbid(unsafe_code)]

mod error;
mod protocol;
mod read;
mod roots;
mod server;

pub use error::{
    ToolError, ToolErrorCode, error_code_i32, invalid_envelope_error, invalid_params_error,
    method_not_found_error, not_initialized_error,
};
pub use protocol::{
    AGENT_NAME, AGENT_TITLE, AGENT_VERSION, PROMPT_REQUEST_SHAPE, PromptToolRequest,
    READ_TOOL_NAME, read_available_commands,
};
pub use read::{
    DEFAULT_LIMIT, MAX_CONTENT_BYTES, MAX_LIMIT, ReadInput, ReadOutput, execute_read,
    format_display_line, format_read_output, line_number_width, parse_read_input,
};
pub use roots::ReadRoots;
pub use server::{ScvServer, help_text, parse_cli_args, serve};
