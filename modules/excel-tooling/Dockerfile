FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libreoffice-calc \
        python3 \
        python3-uno \
        python3-pip \
        python3-venv \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/excel-tooling
COPY pyproject.toml ./
COPY excel_tooling ./excel_tooling
COPY tests ./tests

RUN python3 -m venv --system-site-packages /opt/venv \
    && /opt/venv/bin/pip install --upgrade pip \
    && /opt/venv/bin/pip install --no-cache-dir -e ".[dev]"

COPY docker/libreoffice-profile /opt/libreoffice-profile
COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 50051
ENV PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["/entrypoint.sh"]
CMD ["python", "-m", "excel_tooling"]
