// HTTP route imports — TypeScript client
const users = await fetch("/api/users");
const created = await fetch("/api/users", { method: "POST", body: JSON.stringify(newUser) });
const deleted = await fetch("/api/users/42", { method: "DELETE" });
