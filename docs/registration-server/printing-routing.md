# Printer routing

How a print job chooses a device. The stored fields are [printing.md](printing.md). A pick reads the current printer list from [Finding](printing.md#finding). Viewers subscribe to that same list. The searching list is internal to the aggregator and is not a routing source. The devices on the current list come from [printing-implementation-plan.md](printing-implementation-plan.md) steps 12 and 13. This routing is implemented in step 15 of that plan. Step 19 holds PDF pages, and a zone uses the one window defined here.

A printer can take a job when it is connected, installed, and used. An explicit queue name returns that printer when it can take a job.

## What the job carries

The job names the visitor category. The operator UI can mark the job with a special attribute. Print certificate is that attribute: the visitor screen's "Заказать сертификат". The job can also name a zone. A caller can name a queue.

`is_internet` on a printer stays a filter. A printer with that flag set stays only when the flag equals the job's internet flag. A printer with the flag empty stays only when no used printer has that flag.

## Category

A printer whose routing `category` is set prints that category. A job for the category goes to that printer.

A printer with no category is not a second choice while a used printer has the category.

## Certificate

A job with the certificate attribute goes to a printer whose routing `print_cert` is set. That printer is the certificate printer. The category printer is not used for that job.

The layout for that printer is still the certificate layout: the visitor category plus 1000, then 999, then `-1`, then the smallest non-negative prepared category. The attribute chooses the printer. The layout follows the printer.

When several certificate printers can take the job, and the job names a zone, the zone rule below chooses among the certificate printers in that zone. One certificate printer, with no zone on the job, is that printer.

## Zones

A zone is a name on the printer's routing `zone`. A zone has one or more printers. A job sent to a zone is given to one printer in that zone.

The zone has one rule:

| Rule | Choice |
|---|---|
| `round_robin` | The printer in the zone with the oldest `last_print`. A successful choice stores `last_print` on that printer. |
| `fewest_jobs` | The printer whose device queue has fewer jobs. The count is the queue the device reports. A tie uses the oldest `last_print`. |

A missing rule is `round_robin`. Every printer in the zone carries the same rule. A printer bound to another zone is not a candidate.

## PDF window

A PDF page is joined with the other PDF pages for the same destination. ZPL and PNG are sent as they are drawn. They do not enter a window.

A zone has one sliding window. Pages for that zone share it, including when the zone rule would have chosen different printers for those pages. The window closes after `SLIDING_WINDOW_TIME` of quiet, the same duration as step 19. The flush is one PDF, in arrival order. That document is sent to one printer in the zone, chosen by the zone rule at flush time.

A job that was not sent to a zone keeps one window on the printer step 15 chose. Two such printers do not share pages.
