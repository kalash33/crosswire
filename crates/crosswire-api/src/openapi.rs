//! The OpenAPI document, generated from the annotated routes.

use utoipa::openapi::{InfoBuilder, OpenApi};

/// Builds the OpenAPI document for the whole API surface.
#[must_use]
pub fn document() -> OpenApi {
    let mut openapi = crate::app::api_routes().into_openapi();
    openapi.info = InfoBuilder::new()
        .title("Crosswire API")
        .version("1")
        .description(Some(
            "REST and WebSocket API for the Crosswire sidecar backend.",
        ))
        .build();
    openapi
}
