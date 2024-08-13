use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use serde::{Deserialize, Serialize};
use xsu_dataman::DefaultReturn;

/// A question structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Question {
    /// The author of the question; "anonymous" marks the question as an anonymous question
    pub author: String,
    /// The recipient of the question; cannot be anonymous
    pub recipient: String,
    /// The content of the question
    pub content: String,
    /// The ID of the question
    pub id: String,
    /// The time this question was asked
    pub timestamp: u128,
}

/// A response structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct QuestionResponse {
    /// The author of the response; cannot be anonymous
    pub author: String,
    /// ID of the question this response is replying to
    pub question: Question,
    /// The content of the response
    pub content: String,
    /// The ID of the response
    pub id: String,
    /// The time this response was created
    pub timestamp: u128,
}

// props

#[derive(Serialize, Deserialize, Debug)]
pub struct QuestionCreate {
    pub recipient: String,
    pub content: String,
    pub anonymous: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ResponseCreate {
    pub question: String,
    pub content: String,
}

/// General API errors
pub enum DatabaseError {
    NotAllowed,
    ValueError,
    NotFound,
    Other,
}

impl DatabaseError {
    pub fn to_string(&self) -> String {
        use DatabaseError::*;
        match self {
            NotAllowed => String::from("You are not allowed to do this!"),
            ValueError => String::from("One of the field values given is invalid!"),
            NotFound => String::from("Nothing with this ID could be found!"),
            _ => String::from("An unspecified error has occured"),
        }
    }
}

impl IntoResponse for DatabaseError {
    fn into_response(self) -> Response {
        use DatabaseError::*;
        match self {
            NotAllowed => (
                StatusCode::UNAUTHORIZED,
                Json(DefaultReturn::<u16> {
                    success: false,
                    message: self.to_string(),
                    payload: 401,
                }),
            )
                .into_response(),
            NotFound => (
                StatusCode::NOT_FOUND,
                Json(DefaultReturn::<u16> {
                    success: false,
                    message: self.to_string(),
                    payload: 404,
                }),
            )
                .into_response(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(DefaultReturn::<u16> {
                    success: false,
                    message: self.to_string(),
                    payload: 500,
                }),
            )
                .into_response(),
        }
    }
}

impl<T: Default> Into<xsu_dataman::DefaultReturn<T>> for DatabaseError {
    fn into(self) -> xsu_dataman::DefaultReturn<T> {
        DefaultReturn {
            success: false,
            message: self.to_string(),
            payload: T::default(),
        }
    }
}
