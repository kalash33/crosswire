//! The relay over plain HTTP: absolute-form requests to the proxy,
//! rewritten to origin-form for a scripted TCP upstream.

mod common;

use common::relay::{self as s, Via};

#[tokio::test]
async fn get_round_trip_is_byte_exact() {
    s::get_round_trip_is_byte_exact(Via::Plain).await;
}

#[tokio::test]
async fn keep_alive_two_requests_same_upstream_connection() {
    s::keep_alive_two_requests_same_upstream_connection(Via::Plain).await;
}

#[tokio::test]
async fn connection_close_from_server_ends_relay() {
    s::connection_close_from_server_ends_relay(Via::Plain).await;
}

#[tokio::test]
async fn pipelined_requests_are_served_serially_in_order() {
    s::pipelined_requests_are_served_serially_in_order(Via::Plain).await;
}

#[tokio::test]
async fn request_body_content_length_forwarded_exactly() {
    s::request_body_content_length_forwarded_exactly(Via::Plain).await;
}

#[tokio::test]
async fn chunked_response_with_extensions_and_trailers_is_byte_exact() {
    s::chunked_response_with_extensions_and_trailers_is_byte_exact(Via::Plain).await;
}

#[tokio::test]
async fn head_and_304_responses_are_framed_without_body() {
    s::head_and_304_responses_are_framed_without_body(Via::Plain).await;
}

#[tokio::test]
async fn http10_until_close_response_ends_connection() {
    s::http10_until_close_response_ends_connection(Via::Plain).await;
}

#[tokio::test]
async fn expect_100_continue_interim_is_forwarded_and_recorded() {
    s::expect_100_continue_interim_is_forwarded_and_recorded(Via::Plain).await;
}

#[tokio::test]
async fn host_header_differs_from_authority_is_forwarded_unchanged() {
    s::host_header_differs_from_authority_is_forwarded_unchanged(Via::Plain).await;
}

#[tokio::test]
async fn handler_drop_closes_without_forwarding() {
    s::handler_drop_closes_without_forwarding(Via::Plain).await;
}

#[tokio::test]
async fn pipelining_depth_over_limit_closes() {
    s::pipelining_depth_over_limit_closes(Via::Plain).await;
}
