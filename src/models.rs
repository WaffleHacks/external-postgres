use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorResponse {
    pub code: u16,
    pub message: String,
}

pub mod database {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize)]
    pub struct CreateRequest {
        pub name: String,
        pub password: String,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct DeleteOptions {
        pub retain: Option<bool>,
    }
}

pub mod operator {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize)]
    pub struct StateResponse {
        pub running: bool,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct ChangeStateRequest {
        pub desired: Status,
    }

    #[derive(Clone, Copy, Debug, Deserialize, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Status {
        Enabled,
        Disabled,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct ChangeStateResponse {
        pub success: bool,
    }
}
