"""Serve excel-tooling on port 50051."""

from excel_tooling.server import serve


def main() -> None:
    server, _port, _service = serve("0.0.0.0:50051")
    server.wait_for_termination()


if __name__ == "__main__":
    main()
