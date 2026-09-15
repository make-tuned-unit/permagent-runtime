from registry import register


def test_register_stores_widget_by_id():
    store = {}
    widget = register("abc123", store)
    assert widget["id"] == "abc123"
    assert store["abc123"] == widget
