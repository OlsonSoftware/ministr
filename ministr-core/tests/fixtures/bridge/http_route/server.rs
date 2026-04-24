// HTTP route exports — Rust server (actix-web style)
#[get("/api/users")]
async fn list_users() -> impl Responder {
    HttpResponse::Ok().json(users)
}

#[post("/api/users")]
async fn create_user(body: Json<NewUser>) -> impl Responder {
    HttpResponse::Created()
}

#[delete("/api/users/{id}")]
async fn delete_user(path: Path<u64>) -> impl Responder {
    HttpResponse::NoContent()
}
