#ifndef PDF_TEMPLATE_H
#define PDF_TEMPLATE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PdfTemplate PdfTemplate;

typedef struct PdfTemplateValue {
    const char *field;
    const char *value;
} PdfTemplateValue;

/* Returns 0 on success. On failure returns non-zero and sets pdf_template_last_error. */
int pdf_template_index_bytes(
    const uint8_t *yaml,
    size_t len,
    const char *base_dir,
    PdfTemplate **out);

size_t pdf_template_field_count(const PdfTemplate *t);
const char *pdf_template_field_name(const PdfTemplate *t, size_t index);

int pdf_template_render(
    const PdfTemplate *t,
    const PdfTemplateValue *values,
    const size_t *row_lengths,
    size_t row_count,
    uint8_t **out_bytes,
    size_t *out_len);

void pdf_template_bytes_free(uint8_t *bytes, size_t len);
void pdf_template_free(PdfTemplate *t);
const char *pdf_template_last_error(void);

#ifdef __cplusplus
}
#endif

#endif
