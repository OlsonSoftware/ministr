use fixture::router::dispatch_request;

#[test]
fn dispatches_ping() {
    assert_eq!(dispatch_request(&[7]), vec![0, 7]);
}
