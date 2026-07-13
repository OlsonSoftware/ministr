pub fn dispatch_request_vendor(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().rev().copied().collect()
}
