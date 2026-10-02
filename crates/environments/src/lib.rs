//! Independent secrets broker. No dependency on capture, knowledge, or MCP.
pub mod import;
pub mod openbao;
pub mod process;
pub mod profiles;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid profile or input")]
    Invalid,
    #[error("environment access denied")]
    Denied,
    #[error("vault unavailable or request rejected")]
    Vault,
    #[error("local operation failed")]
    Local,
    #[error("secure export unsupported on this platform")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;
