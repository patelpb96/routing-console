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
use tokio::sync::watch;
use tokio_stream::Stream;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::{BroadcastStream, WatchStream};

use crate::app::{App, CommandInput, CreateTask};
use crate::error::AppError;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const SIM_JS: &str = include_str!("../web/sim.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

type Shared = State<Arc<App>>;

/// Turns true once shutdown begins; every open event stream ends when it does.
type Closing = axum::Extension<watch::Receiver<bool>>;

fn router(app: Arc<App>, closing: watch::Receiver<bool>) -> Router {
    Router::new()
        .route("/", get(|| page("text/html; charset=utf-8", INDEX_HTML)))
        .route(
            "/app.js",
            get(|| page("text/javascript; charset=utf-8", APP_JS)),
        )
        .route(
            "/sim.js",
            get(|| page("text/javascript; charset=utf-8", SIM_JS)),
        )
        .route(
            "/style.css",
            get(|| page("text/css; charset=utf-8", STYLE_CSS)),
        )
        .route("/api/snapshot", get(snapshot))
        .route("/api/events", get(events))
        .route("/api/overrides.jsonl", get(overrides))
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/{id}/commands", post(command))
        .route("/api/sim/assignments/{id}/reply", post(reply))
        .route("/api/sim/tasks/{id}/scenario", post(scenario))
        .route("/api/sim/reset", post(reset))
        .layer(axum::Extension(closing))
        .with_state(app)
}

/// Serves the app on `listener` until `shutdown` completes. Then it stops taking connections,
/// ends every open event stream (they never finish on their own), and waits for the requests
/// still in flight to finish.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Arc<App>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let (close, closing) = watch::channel(false);
    axum::serve(listener, router(app, closing))
        .with_graceful_shutdown(async move {
            shutdown.await;
            let _ = close.send(true);
        })
        .await
}

async fn page(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

async fn snapshot(State(app): Shared) -> Result<Response, AppError> {
    Ok(Json(app.snapshot()?).into_response())
}

async fn overrides(State(app): Shared) -> Result<Response, AppError> {
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"overrides.jsonl\"",
            ),
        ],
        app.overrides_jsonl()?,
    )
        .into_response())
}

async fn events(
    State(app): Shared,
    axum::Extension(closing): Closing,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // A lagged receiver missed some signals, which still means "changed", so every item counts.
    let changes = BroadcastStream::new(app.subscribe()).map(|_| Some(()));
    // One `None` once shutdown has begun; the watch stream yields the current value first, so a
    // stream opened after that ends at once.
    let close = WatchStream::new(closing).filter_map(|closed| closed.then_some(None));
    let stream = changes
        .merge(close)
        .take_while(Option::is_some)
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ResetBody {
    seed: Option<u64>,
}

/// The body is optional: no body, an empty body and `{}` all keep the stored seed.
async fn reset(State(app): Shared, body: axum::body::Bytes) -> Result<Response, AppError> {
    let body: ResetBody = if body.iter().all(u8::is_ascii_whitespace) {
        ResetBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::Refused(format!("Bad request body: {e}")))?
    };
    Ok(Json(app.reset(body.seed)?).into_response())
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
