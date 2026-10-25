#ifndef NATIVE_PRINT_H
#define NATIVE_PRINT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NativePrintList NativePrintList;

/* The same calls on Linux, macOS, and Windows. service NULL uses the local
   print service. A non-NULL service is the address of that service.
   Returns 0 on success. On failure returns non-zero, sets native_print_last_error,
   and does not set the out-pointer. The caller frees a list once. */

int native_print_list(const char *service, NativePrintList **out, size_t *count);
void native_print_list_free(NativePrintList *list);
const char *native_print_name(const NativePrintList *list, size_t index);
const char *native_print_device_uri(const NativePrintList *list, size_t index);
const char *native_print_make_model(const NativePrintList *list, size_t index);
const char *native_print_state(const NativePrintList *list, size_t index);
const char *native_print_host(const NativePrintList *list, size_t index);
const char *native_print_kind(const NativePrintList *list, size_t index);
const char *native_print_width_in(const NativePrintList *list, size_t index);
int native_print_accepting(const NativePrintList *list, size_t index);
int native_print_dpi(const NativePrintList *list, size_t index);
int native_print_installed(const NativePrintList *list, size_t index);
int native_print_connected(const NativePrintList *list, size_t index);
size_t native_print_choice_count(const NativePrintList *list, size_t index);
const char *native_print_choice_name(const NativePrintList *list, size_t index, size_t choice);
size_t native_print_choice_value_count(const NativePrintList *list, size_t index, size_t choice);
const char *native_print_choice_value(const NativePrintList *list, size_t index, size_t choice, size_t value);
const char *native_print_last_error(void);

#ifdef __cplusplus
}
#endif

#endif
