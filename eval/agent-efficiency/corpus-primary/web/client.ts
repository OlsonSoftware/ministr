export async function dispatch(payload: Uint8Array): Promise<Response> {
  return fetch("/dispatch", { method: "POST", body: payload });
}
