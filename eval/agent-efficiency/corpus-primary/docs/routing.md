# Routing

## Dispatch contract

The production dispatcher chooses a handler from an inbound wire envelope and
emits the reply. Tests exercise the same dispatcher through a ping frame. The
HTTP client reaches it through `POST /dispatch`.
