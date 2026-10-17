//! Engine errors. Every variant renders to an i18n [`Message`].

use udm_core::Message;
use udm_net::NetError;
use udm_proto::ProtoError;
use udm_store::StoreError;
use udm_sys::SysError;

/// Engine error.
#[derive(Debug)]
pub enum EngineError {
    /// A message produced directly by engine logic (validation, state).
    Msg(Message),
    /// Network.
    Net(NetError),
    /// Protocol parsing.
    Proto(ProtoError),
    /// Storage.
    Store(StoreError),
    /// System (disk, power, clipboard).
    Sys(SysError),
}

impl EngineError {
    /// i18n message.
    pub fn to_message(&self) -> Message {
        match self {
            EngineError::Msg(m) => m.clone(),
            EngineError::Net(e) => e.to_message(),
            EngineError::Proto(e) => e.to_message(),
            EngineError::Store(e) => e.to_message(),
            EngineError::Sys(e) => e.to_message(),
        }
    }

    /// Shorthand for a key-only message.
    pub fn key(key: &str) -> Self {
        EngineError::Msg(Message::new(key))
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_message())
    }
}

impl std::error::Error for EngineError {}

impl From<Message> for EngineError {
    fn from(m: Message) -> Self {
        EngineError::Msg(m)
    }
}

impl From<NetError> for EngineError {
    fn from(e: NetError) -> Self {
        EngineError::Net(e)
    }
}

impl From<ProtoError> for EngineError {
    fn from(e: ProtoError) -> Self {
        EngineError::Proto(e)
    }
}

impl From<StoreError> for EngineError {
    fn from(e: StoreError) -> Self {
        EngineError::Store(e)
    }
}

impl From<SysError> for EngineError {
    fn from(e: SysError) -> Self {
        EngineError::Sys(e)
    }
}

/// Result alias.
pub type EngineResult<T> = Result<T, EngineError>;
