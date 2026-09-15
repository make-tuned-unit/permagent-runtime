def call_api(fetch, *args, **kwargs):
    """Call the API client's fetch function once. No retry policy yet."""
    return fetch(*args, **kwargs)
