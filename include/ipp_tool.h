#ifndef IPP_TOOL_H
#define IPP_TOOL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct IppPrinter IppPrinter;
typedef struct IppJob IppJob;

/* Returns 0 on success. On failure returns non-zero and sets ipp_tool_last_error.
   On failure an out-pointer is not set. */

int ipp_tool_printer_open(const char *uri, IppPrinter **out);
int ipp_tool_info(IppPrinter *printer);
const char *ipp_tool_make_model(const IppPrinter *printer);
int ipp_tool_set(IppPrinter *printer, const char *setting, const char *value);
int ipp_tool_print_test(IppPrinter *printer, int *job_id);
int ipp_tool_print_file(IppPrinter *printer, const char *path, int raw, int *job_id);
int ipp_tool_jobs(IppPrinter *printer, IppJob **out, size_t *count);
int ipp_tool_cancel(IppPrinter *printer, int job_id);
int ipp_tool_hold(IppPrinter *printer, int job_id);
int ipp_tool_release(IppPrinter *printer, int job_id);
int ipp_tool_discover(IppPrinter ***out, size_t *count);
int ipp_tool_refresh(IppPrinter ***printers, size_t *count);
int ipp_tool_add(const char *uri, IppPrinter **out);
void ipp_tool_printer_free(IppPrinter *printer);
void ipp_tool_list_free(IppPrinter **list, size_t count);
const char *ipp_tool_last_error(void);

#ifdef __cplusplus
}
#endif

#endif
