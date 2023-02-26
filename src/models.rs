pub mod database {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize)]
    pub struct CreateRequest {
        pub name: String,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct CreateResponse {
        pub password: Option<String>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct DeleteOptions {
        pub retain: Option<bool>,
    }
}
