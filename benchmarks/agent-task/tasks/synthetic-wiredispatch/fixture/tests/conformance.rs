//! Wire-protocol conformance suite. Operation codes here mirror the
//! released v0.4 client libraries; they are the protocol's public
//! contract and MUST NOT be edited to make tests pass.
use relaykit::transport::LoopbackClient;

fn call(op: u16, payload: &[u8]) -> (u8, Vec<u8>) {
    let reply = LoopbackClient::new()
        .call(op, payload)
        .unwrap_or_else(|| panic!("op 0x{op:04X}: no route"));
    (reply.kind, reply.body)
}

#[test]
fn op_0001_conformance() {
    let (kind, body) = call(0x0001, &[7]);
    assert_eq!(kind, 1, "op 0x0001 reply kind");
    assert_eq!(body, vec![8], "op 0x0001 body");
}

#[test]
fn op_0002_conformance() {
    let (kind, body) = call(0x0002, &[1, 2, 3]);
    assert_eq!(kind, 2, "op 0x0002 reply kind");
    assert_eq!(body, vec![3], "op 0x0002 body");
}

#[test]
fn op_0003_conformance() {
    let (kind, body) = call(0x0003, &[9]);
    assert_eq!(kind, 3, "op 0x0003 reply kind");
    assert_eq!(body, vec![0xAA, 9], "op 0x0003 body");
}

#[test]
fn op_0004_conformance() {
    let (kind, body) = call(0x0004, &[3, 5]);
    assert_eq!(kind, 4, "op 0x0004 reply kind");
    assert_eq!(body, vec![0], "op 0x0004 body");
}

#[test]
fn op_0005_conformance() {
    let (kind, body) = call(0x0005, &[1, 2]);
    assert_eq!(kind, 5, "op 0x0005 reply kind");
    assert_eq!(body, vec![1, 2], "op 0x0005 body");
}

#[test]
fn op_0006_conformance() {
    let (kind, body) = call(0x0006, &[10, 20]);
    assert_eq!(kind, 6, "op 0x0006 reply kind");
    assert_eq!(body, vec![2, 0x4A], "op 0x0006 body");
}

#[test]
fn op_0007_conformance() {
    let (kind, body) = call(0x0007, &[14]);
    assert_eq!(kind, 7, "op 0x0007 reply kind");
    assert_eq!(body, vec![50], "op 0x0007 body");
}

#[test]
fn op_0008_conformance() {
    let (kind, body) = call(0x0008, &[1, 2, 3]);
    assert_eq!(kind, 8, "op 0x0008 reply kind");
    assert_eq!(body, vec![0], "op 0x0008 body");
}

#[test]
fn unknown_op_is_unrouted() {
    assert!(LoopbackClient::new().call(0x00FF, &[]).is_none());
}
