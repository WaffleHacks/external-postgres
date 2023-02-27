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
