from widget import make_widget


def register(old_id: str, store: dict) -> dict:
    """Create a widget for old_id and record it in store, keyed by old_id."""
    widget = make_widget(old_id)
    store[old_id] = widget
    return widget
