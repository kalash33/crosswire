//! The relay inside a MITM tunnel: `CONNECT`, TLS with a Crosswire leaf,
//! origin-form requests relayed to a scripted TLS upstream. Runs the same
//! scenarios as `relay_plain.rs`.

mod common;

use common::relay::{self as s, read_n, Via};
use tokio::io::AsyncWriteExt;

async fn run_tls_scenario<F, Fut>(scenario: F)
where
    F: FnOnce(Via) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    scenario(Via::Tls).await;
}

#[tokio::test]
async fn get_round_trip_is_byte_exact() {
    run_tls_scenario(s::get_round_trip_is_byte_exact).await;
}

#[tokio::test]
async fn keep_alive_two_requests_same_upstream_connection() {
    run_tls_scenario(s::keep_alive_two_requests_same_upstream_connection).await;
}

#[tokio::test]
async fn connection_close_from_server_ends_relay() {
    run_tls_scenario(s::connection_close_from_server_ends_relay).await;
}

#[tokio::test]
async fn pipelined_requests_are_served_serially_in_order() {
    run_tls_scenario(s::pipelined_requests_are_served_serially_in_order).await;
}

#[tokio::test]
async fn request_body_content_length_forwarded_exactly() {
    run_tls_scenario(s::request_body_content_length_forwarded_exactly).await;
}

#[tokio::test]
async fn chunked_response_with_extensions_and_trailers_is_byte_exact() {
    run_tls_scenario(s::chunked_response_with_extensions_and_trailers_is_byte_exact).await;
}

#[tokio::test]
async fn head_and_304_responses_are_framed_without_body() {
    run_tls_scenario(s::head_and_304_responses_are_framed_without_body).await;
}

#[tokio::test]
async fn http10_until_close_response_ends_connection() {
    run_tls_scenario(s::http10_until_close_response_ends_connection).await;
}

#[tokio::test]
async fn expect_100_continue_interim_is_forwarded_and_recorded() {
    run_tls_scenario(s::expect_100_continue_interim_is_forwarded_and_recorded).await;
}

#[tokio::test]
async fn host_header_differs_from_authority_is_forwarded_unchanged() {
    run_tls_scenario(s::host_header_differs_from_authority_is_forwarded_unchanged).await;
}

#[tokio::test]
async fn handler_drop_closes_without_forwarding() {
    run_tls_scenario(s::handler_drop_closes_without_forwarding).await;
}

#[tokio::test]
async fn pipelining_depth_over_limit_closes() {
    run_tls_scenario(s::pipelining_depth_over_limit_closes).await;
}

/// `CONNECT 127.0.0.1:<port>`: the leaf and the upstream verification both
/// use the IP (the upstream's certificate has only an IP SAN).
#[tokio::test]
async fn connect_to_ip_literal_relays() {
    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nip".to_vec();
    let rig = s::rig(Via::Tls, vec![resp.clone()], None, &[]).await;
    let mut c = rig.open("127.0.0.1").await;
    let req = format!(
        "{}\r\nHost: 127.0.0.1:{}\r\n\r\n",
        rig.start_line("GET", "127.0.0.1", "/ip"),
        rig.port
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].request().connection().target_addr(), "127.0.0.1");
    assert_eq!(rig.connections(), 1);
}
