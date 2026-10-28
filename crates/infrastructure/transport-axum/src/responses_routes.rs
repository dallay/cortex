use std::{convert::Infallible, sync::Arc};

use crate::responses_stream::ResponsesStream;
use axum::response::{sse::Event, Sse};
use axum::{
    extract::{rejection::JsonRejection, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use rook_core::ApiFormat;
use serde_json::json;

use crate::{
    authz,
    responses_adapter::{ResponseContext, ResponsesRequest},
    routes,
};

pub(crate) async fn create(
    State(usecases): State<Arc<rook_usecases::RookUsecases>>,
    headers: HeaderMap,
    body: Result<Json<ResponsesRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(body)) => body.into_domain(),
        Err(error) => {
            let status = match error.status() {
                StatusCode::UNSUPPORTED_MEDIA_TYPE => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                StatusCode::PAYLOAD_TOO_LARGE => StatusCode::PAYLOAD_TOO_LARGE,
                _ => StatusCode::BAD_REQUEST,
            };
            return failure(
                status,
                "invalid_request_error",
                "Solicitud Responses inválida o con campos no soportados",
            );
        }
    };
    let mut request = match request {
        Ok(request) => request,
        Err(message) => return failure(StatusCode::BAD_REQUEST, "invalid_request_error", &message),
    };
    request.restrictions = match routes::restrictions_from_headers(&headers) {
        Ok(restrictions) => restrictions,
        Err(error) => return failure(error.status, error.code, &error.message),
    };
    request.metadata.api_key_id = authz::extract_api_key_id_from_headers(&headers);
    request.metadata.combo_id =
        match routes::combo_id_from_headers(&headers, request.model.as_str()) {
            Ok(combo) => combo,
            Err(error) => return failure(error.status, error.code, &error.message),
        };
    let context = ResponseContext::new(&request);
    let route_request = usecases
        .route_request
        .clone()
        .with_output_validation(Arc::new(ResponsesValidation(context.clone())));
    if request.stream {
        let stream = match route_request
            .execute_stream_with_format(request, ApiFormat::OpenAI)
            .await
        {
            Ok(stream) => stream,
            Err(error) => return route_failure(&error),
        };
        let mut encoder = ResponsesStream::new(context);
        let initial = encoder.start();
        let events = futures::stream::iter(initial)
            .chain(
                futures::stream::unfold(Some((stream, encoder)), |state| async move {
                    let (mut stream, mut encoder) = state?;
                    let (events, next) = match stream.next().await {
                        Some(Ok(chunk)) => match encoder.push(chunk) {
                            Ok(events) => (events, Some((stream, encoder))),
                            Err(message) => (encoder.fail(message), None),
                        },
                        Some(Err(error)) => {
                            if let Some(usage) = rook_core::StreamValidationError::usage(&error) {
                                encoder.observe_usage(usage.clone());
                            }
                            (encoder.fail("El stream upstream falló"), None)
                        }
                        None => (
                            match encoder.finish() {
                                Ok(events) => events,
                                Err(message) => encoder.fail(message),
                            },
                            None,
                        ),
                    };
                    Some((events, next))
                })
                .flat_map(futures::stream::iter),
            )
            .map(|event| {
                Ok::<Event, Infallible>(
                    Event::default()
                        .event(event["type"].as_str().unwrap_or("error"))
                        .data(event.to_string()),
                )
            });
        let mut response = Sse::new(events).into_response();
        response.headers_mut().insert(
            "cache-control",
            axum::http::HeaderValue::from_static("no-cache"),
        );
        return response;
    }
    match route_request
        .execute_with_format(request, ApiFormat::OpenAI)
        .await
    {
        Ok(response) => match context.response_body(&response) {
            Ok(body) => Json(body).into_response(),
            Err(_) => failure(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "Respuesta de función inválida",
            ),
        },
        Err(error) => route_failure(&error),
    }
}

struct ResponsesValidation(ResponseContext);

impl rook_usecases::route_request::OutputValidation for ResponsesValidation {
    fn response(
        &self,
        response: &rook_core::CompletionResponse,
    ) -> Result<(), rook_core::CortexError> {
        self.0
            .response_body(response)
            .map(|_| ())
            .map_err(rook_core::CortexError::provider)
    }

    fn stream(&self) -> Box<dyn rook_usecases::route_request::StreamOutputValidation> {
        Box::new(ResponsesStream::new(self.0.clone()))
    }
}

impl rook_usecases::route_request::StreamOutputValidation for ResponsesStream {
    fn chunk(&mut self, chunk: &rook_core::StreamChunk) -> Result<(), rook_core::CortexError> {
        self.push(chunk.clone()).map(|_| ()).map_err(|message| {
            rook_core::StreamValidationError::attach(
                rook_core::CortexError::provider(message),
                chunk.usage.clone(),
            )
        })
    }

    fn finish(&mut self) -> Result<(), rook_core::CortexError> {
        ResponsesStream::finish(self)
            .map(|_| ())
            .map_err(rook_core::CortexError::provider)
    }
}

fn route_failure(error: &shared_kernel::CortexError) -> Response {
    let (status, code) = if error.is_forbidden() {
        (StatusCode::FORBIDDEN, "permission_denied")
    } else if error.is_not_found() {
        (StatusCode::NOT_FOUND, "model_not_found")
    } else if error.is_rate_limited() {
        (StatusCode::TOO_MANY_REQUESTS, "rate_limit_exceeded")
    } else if error.is_invalid_request() {
        (StatusCode::BAD_REQUEST, "invalid_request_error")
    } else {
        (StatusCode::BAD_GATEWAY, "upstream_error")
    };
    let mut response = failure(status, code, "No se pudo completar la solicitud");
    if error.is_rate_limited() {
        if let Ok(value) = error.retry_after_secs().unwrap_or(60).to_string().parse() {
            response.headers_mut().insert("retry-after", value);
        }
    }
    response
}

fn failure(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error":{"type":code, "code":code, "message":message, "param":null}})),
    )
        .into_response()
}
