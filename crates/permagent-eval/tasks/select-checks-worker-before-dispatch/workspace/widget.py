def make_widget(old_id: str) -> dict:
    """Build a widget record keyed by its identifier."""
    return {"id": old_id, "label": f"widget-{old_id}"}
