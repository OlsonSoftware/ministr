# HTTP route exports — Python server (FastAPI style)
@app.get("/api/items")
def list_items():
    return items

@app.post("/api/items")
def create_item(item: Item):
    return {"id": 1, **item.dict()}

@app.delete("/api/items/{item_id}")
def delete_item(item_id: int):
    return {"deleted": item_id}
