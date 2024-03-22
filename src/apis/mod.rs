use serde::Deserialize;

pub mod tools;

#[derive(Deserialize)]
pub struct GetReq {
    pub id: u32,
}
