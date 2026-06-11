//! v0.3 routes mapped operations by NAME on the wire. Retired because
//! name strings dominated frame size. Kept for the migration shim.
#[allow(dead_code)]
pub const OLD_ROUTES: &[(&str, &str)] = &[
    ("link.open", "session"),
    ("who.is.here", "presence"),
    ("pulse", "beacon"),
    ("entry.post", "ledger"),
    ("echo", "mirror"),
    ("record.add", "journal"),
    ("balance", "quota"),
    ("window.fold", "digest"),
];
