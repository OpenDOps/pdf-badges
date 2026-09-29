import pytest


def pytest_collection_modifyitems(config, items):
    try:
        import uno  # noqa: F401
    except ImportError:
        skip = pytest.mark.skip(reason="LibreOffice Python bridge is not installed")
        for item in items:
            if "libreoffice" in item.keywords:
                item.add_marker(skip)
