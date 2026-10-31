FROM rust:1-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler musl-tools ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add "$(rustc -vV | sed -n 's/^host: //p' | sed 's/-gnu$/-musl/')"
ENV CC_aarch64_unknown_linux_musl=musl-gcc \
    CC_x86_64_unknown_linux_musl=musl-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY proto ./proto
COPY modules/google-tables ./modules/google-tables
COPY modules/ticket-render/Cargo.toml modules/ticket-render/Cargo.toml
COPY modules/ipp-tool/Cargo.toml modules/ipp-tool/Cargo.toml

# Cargo loads every workspace member. The other crates are not compiled.
RUN mkdir -p src/bin modules/ticket-render/src modules/ipp-tool/src tests \
    && : > src/lib.rs \
    && : > src/main.rs \
    && : > src/bin/rust-reg-run.rs \
    && : > modules/ticket-render/src/lib.rs \
    && : > modules/ipp-tool/src/lib.rs \
    && : > tests/live_remote.rs \
    && : > tests/registration_server.rs \
    && : > tests/struct_to_pdf.rs \
    && : > tests/pdf_grpc.rs \
    && : > tests/admin_pages.rs \
    && : > tests/template.rs

RUN target="$(rustc -vV | sed -n 's/^host: //p' | sed 's/-gnu$/-musl/')" \
    && cargo build --release --locked --target "$target" -p google-tables \
    && cp "target/${target}/release/google-tables" /tmp/google-tables

RUN cat > /tmp/tcpcheck.c <<'EOF'
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

int main(void) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {
        return 1;
    }
    struct timeval tv = {.tv_sec = 2, .tv_usec = 0};
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(50052);
    addr.sin_addr.s_addr = htonl(0x7f000001);
    int rc = connect(fd, (struct sockaddr *)&addr, sizeof addr);
    close(fd);
    return rc == 0 ? 0 : 1;
}
EOF
RUN musl-gcc -static -O2 -o /tmp/tcpcheck /tmp/tcpcheck.c

FROM scratch

COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /tmp/google-tables /google-tables
COPY --from=build /tmp/tcpcheck /tcpcheck

EXPOSE 50052
ENTRYPOINT ["/google-tables"]
