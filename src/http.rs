//! The routes of contract section 5. Handlers only translate: every rule lives in `app` and
//! `domain`, and every failure leaves through `AppError`'s one mapping to a status code.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Json, Path, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;
use tokio_stream::Stream;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::app::{App, CommandInput, CreateTask};
use crate::error::AppError;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

type Shared = State<Arc<App>>;

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(|| page("text/html; charset=utf-8", INDEX_HTML)))
        .route(
            "/app.js",
            get(|| page("text/javascript; charset=utf-8", APP_JS)),
        )
        .route(
            "/style.css",
            get(|| page("text/css; charset=utf-8", STYLE_CSS)),
        )
        .route("/api/snapshot", get(snapshot))
        .route("/api/events", get(events))
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/{id}/commands", post(command))
        .route("/api/sim/assignments/{id}/reply", post(reply))
        .route("/api/sim/tasks/{id}/scenario", post(scenario))
        .route("/api/sim/reset", post(reset))
        .with_state(app)
}

async fn page(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

async fn snapshot(State(app): Shared) -> Result<Response, AppError> {
    Ok(Json(app.snapshot()?).into_response())
}

async fn events(State(app): Shared) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // A lagged receiver missed some signals, which still means "changed", so every item counts.
    let stream = BroadcastStream::new(app.subscribe())
        .map(|_| Ok(Event::default().event("changed").data("{}")));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn create_task(
    State(app): Shared,
    body: Result<Json<CreateTask>, JsonRejection>,
) -> Result<Response, AppError> {
    let task = app.create_task(json_body(body)?)?;
    Ok((StatusCode::CREATED, Json(task)).into_response())
}

async fn command(
    State(app): Shared,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<CommandInput>, JsonRejection>,
) -> Result<Response, AppError> {
    let task = app.command(path_id(id)?, json_body(body)?)?;
    Ok(Json(task).into_response())
}

#[derive(Deserialize)]
struct ReplyBody {
    reply: String,
}

async fn reply(
    State(app): Shared,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<ReplyBody>, JsonRejection>,
) -> Result<Response, AppError> {
    let task = app.reply(path_id(id)?, &json_body(body)?.reply)?;
    Ok(Json(task).into_response())
}

#[derive(Deserialize)]
struct ScenarioBody {
    scenario: String,
}

async fn scenario(
    State(app): Shared,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<ScenarioBody>, JsonRejection>,
) -> Result<Response, AppError> {
    let task = app.set_scenario(path_id(id)?, &json_body(body)?.scenario)?;
    Ok(Json(task).into_response())
}

async fn reset() -> Result<Response, AppError> {
    Err(AppError::Refused(
        "Simulation reset is not available in this build yet.".into(),
    ))
}

fn json_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, AppError> {
    body.map(|Json(b)| b)
        .map_err(|e| AppError::Refused(format!("Bad request body: {}", e.body_text())))
}

fn path_id(id: Result<Path<i64>, PathRejection>) -> Result<i64, AppError> {
    id.map(|Path(i)| i)
        .map_err(|_| AppError::NotFound("No such id.".into()))
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound(message) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "not_found", "message": message})),
            )
                .into_response(),
            AppError::Refused(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "refused", "message": message})),
            )
                .into_response(),
            AppError::Stale(task) => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "stale_version",
                    "message": format!(
                        "The task is now at version {}; check it and try again.",
                        task.version
                    ),
                    "task": task,
                })),
            )
                .into_response(),
            AppError::Internal(message) => {
                eprintln!("internal error: {message}");
                (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
            }
        }
    }
}
