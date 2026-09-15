from client import call_api


def test_call_api_returns_fetch_result():
    def fetch():
        return "ok"

    assert call_api(fetch) == "ok"
