FROM node:22-bookworm-slim

WORKDIR /app

COPY ticket-editor/package.json ticket-editor/package-lock.json ./
RUN npm ci --omit=dev

COPY ticket-editor/app.mjs ticket-editor/server.mjs ./
COPY proto/irbis/pdf/v1/pdf.proto /app/proto/irbis/pdf/v1/pdf.proto
COPY tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf /app/fonts/LiberationSerif-Regular.ttf

ENV DEFAULT_FONT=/app/fonts/LiberationSerif-Regular.ttf \
    PDF_PROTO=/app/proto/irbis/pdf/v1/pdf.proto \
    PORT=4310

EXPOSE 4310
CMD ["node", "server.mjs"]
