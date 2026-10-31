// Mirrors backend/registration/serializers.py (print templates / printers
// block) and views_print.py payloads — docs/TZ_print_templates.md, раздел 5.6.

export type PrintTemplateSource = 'cloud' | 'local';
export type PageFormat = 'A4' | 'A5' | 'A6' | 'Letter' | 'Legal' | 'custom' | 'roll';
export type Orientation = 'portrait' | 'landscape';

export interface PageFields {
  page_format: PageFormat;
  page_width_mm: string | number | null;
  page_height_mm: string | number | null;
  orientation: Orientation;
  margin_top_mm: string | number;
  margin_right_mm: string | number;
  margin_bottom_mm: string | number;
  margin_left_mm: string | number;
}

export interface PrintTemplateListItem extends PageFields {
  id: number;
  ticket_type: number;
  ticket_type_name: string;
  name: string;
  source: PrintTemplateSource;
  is_default: boolean;
  updated_at: string;
  updated_by_name: string;
}

/** The page object stored in template.yml. JSON on the wire, YAML on disk. */
export interface PrintPageDocument {
  page_size: {
    width: number;
    height: number;
    units?: string;
    bleeds?: { top: number; right: number; bottom: number; left: number };
    background?: string;
  };
  resources?: {
    images?: Record<string, { name?: string; extracted_path?: string }>;
    fonts?: Record<string, { name?: string; font_family?: string; source_path?: string }>;
  };
  contents: Record<string, unknown>[];
}

export interface PrintTemplateDetail extends PrintTemplateListItem {
  html: string | null;
  /** Present once the template is a YAML page. The operator route leaves it unset. */
  page?: PrintPageDocument | null;
  editor_state: unknown | null;
  manifest: Record<string, string>;
  created_at: string;
}

export interface PrintTemplateWrite extends Partial<PageFields> {
  name?: string;
  html?: string;
  /** The page the browser has been editing. The Node process writes it as template.yml. */
  page?: PrintPageDocument | null;
  editor_state?: unknown | null;
  /** Optimistic-lock token — the `updated_at` the editor loaded. */
  updated_at?: string;
}

export interface PrintTemplateCreateRequest {
  ticket_type: number;
  name: string;
  copy_from?: number | null;
}

export interface PageFormatsCatalog {
  standard: { format: PageFormat; width_mm: number; height_mm: number }[];
  thermal_presets: { label: string; width_mm: number; height_mm: number }[];
  custom: { min_mm: number; max_mm: number };
  orientations: Orientation[];
  default_new_template: { format: PageFormat; width_mm: number; height_mm: number };
}

/** `page` block as the preview endpoint and the archive manifest carry it. */
export interface PagePayload {
  format: PageFormat;
  width_mm: number | null;
  height_mm: number | null;
  orientation: Orientation;
  margins_mm: { top: number; right: number; bottom: number; left: number };
}

export interface PreviewRequest {
  template_id?: number | null;
  ticket_type?: number | null;
  html?: string;
  page?: PagePayload;
  /** The page object currently in the editor. Preview draws this, not the saved file. */
  document?: unknown;
  registration_number?: string;
  output?: 'pdf' | 'mono';
  dpi?: number;
}

export type DemoFieldKind =
  | 'text' | 'textarea' | 'choice' | 'multichoice' | 'boolean' | 'phone' | 'email' | 'date';

export interface DemoField {
  key: string;
  label: string;
  kind: DemoFieldKind;
  section: string;
  sample: string | string[];
}

export interface DemoContext {
  fields: DemoField[];
  ticket: { number: string; code: string; ticket_type_name: string };
  form_data: Record<string, string | string[]>;
  has_vcard: boolean;
  codes: Record<'barcode' | 'qrcode' | 'vcard_qrcode' | 'barcode_svg' | 'qrcode_svg' | 'vcard_qrcode_svg', string>;
  geometry: { ean13_modules: number; qr_modules: number };
}

export type DispatchMode = 'zpl_tcp' | 'ipp' | 'lp' | 'file' | 'agent';
/** What the node sends to a Windows print agent (docs/TZ_print_agent.md,
 * раздел 3.1): a raster for any Windows printer, or ZPL straight to a
 * Zebra on USB. */
export type AgentKind = 'bitmap' | 'raw_zpl';
export type AgentFit = 'auto' | 'fit' | 'actual';
export type IppFormat = 'auto' | 'pdf' | 'pwg';
export type IppColorMode = 'auto' | 'monochrome' | 'color';

export interface Printer {
  id: number;
  name: string;
  dispatch_mode: DispatchMode;
  is_default: boolean;
  is_active: boolean;
  host: string;
  port: number;
  dpi: number;
  media_tracking: 'gap' | 'mark' | 'continuous';
  post_print: 'tear' | 'cut' | 'peel';
  print_method: 'transfer' | 'direct';
  darkness: number | null;
  speed: number | null;
  offset_x_mm: string | number;
  offset_y_mm: string | number;
  /** Roll pitch for ZPL `^LL` — NULL means the template page height. Set it
   * when a cutter must cut on a perforation that sits past the printed area. */
  label_length_mm: string | number | null;
  /** ZPL `~TA` — where the media stops (and a cutter cuts) relative to the
   * sensed label, mm; negative is earlier. NULL — keep the printer's value. */
  tear_off_mm: string | number | null;
  check_status: boolean;
  cups_queue: string;
  lp_options: string[];
  /** ipp (docs/TZ_lan_node.md, раздел 5.1) */
  ipp_url: string;
  ipp_format: IppFormat;
  ipp_color_mode: IppColorMode;
  ipp_media_source: string;
  /** agent (docs/TZ_print_agent.md, раздел 3.1) — `dpi` and the offsets
   * are reused from the Zebra block. */
  agent: number | null;
  agent_printer: string;
  agent_kind: AgentKind;
  /** Raster placement on the Windows printer's sheet (applied by the agent):
   * auto = real size unless the ink leaves the printable area; fit = always
   * fit into it; actual = real size even if edges are lost. */
  agent_fit: AgentFit;
  /** Extra margin inside the printable area on every side, mm. */
  agent_margin_mm: string | number;
  updated_at: string;
}

export type PrinterWrite = Omit<Printer, 'id' | 'is_default' | 'updated_at'>;

export interface PrinterStatus {
  online: boolean | null;
  paper_out?: boolean;
  paused?: boolean;
  head_open?: boolean;
  ribbon_out?: boolean;
  buffer_full?: boolean;
  problem?: string | null;
  detail?: string;
  /** agent: seconds since the heartbeat the state comes from. */
  age_s?: number | null;
}

/** `{ "<ticket_type_id>": template_id }` */
export type PrinterBindings = Record<string, number>;

/** `{ "<ticket_type_id>": printer_id }` — node-wide ticket type → printer
 * rules that win over the workstation printer (docs/TZ_lan_node.md, 5.5). */
export type PrinterRoutes = Record<string, number>;

/** `POST printer/ipp_probe` — Get-Printer-Attributes of an IPP address. */
export interface IppProbeResult {
  ok: boolean;
  detail?: string;
  name?: string;
  make_and_model?: string;
  state?: 'idle' | 'processing' | 'stopped' | string;
  accepting_jobs?: boolean;
  problem?: string | null;
  warnings?: string[];
  formats?: string[];
  resolutions?: number[];
  raster_types?: string[];
  media?: string[];
  media_ready?: string[];
  media_sources?: string[];
  color_modes?: string[];
  recommended_format?: string | null;
}

/** Optional target on print-shaped requests (registration print / pdf /
 * reissue): the workstation's printer and a one-off template override. */
export interface PrintTarget {
  printer_id?: number | null;
  template_id?: number | null;
}

// --- Windows print agents (docs/TZ_print_agent.md, разделы 3.3 и 4) ---

/** Windows printer state codes from PRINTER_INFO_2.Status (раздел 4.3). */
export type WindowsPrinterStatusCode =
  | 'paused'
  | 'error'
  | 'paper_out'
  | 'paper_jam'
  | 'offline'
  | 'door_open'
  | 'toner_low'
  | 'no_toner'
  | 'not_available'
  | 'busy'
  | 'printing'
  | 'user_intervention';

export type DiagnosticStatus = 'ok' | 'warn' | 'fail';

export interface PrintAgent {
  id: number;
  name: string;
  hostname: string;
  version: string;
  online: boolean;
  last_seen_at: string | null;
  paused: boolean;
  printers_count: number;
  pairing_code: string | null;
  pairing_expires_at: string | null;
  paired: boolean;
  diagnostics_summary: { ok: number; warn: number; fail: number } | null;
  last_error: string | null;
}

export interface AgentWindowsPrinter {
  name: string;
  port: string;
  driver: string;
  location: string;
  comment: string;
  is_default: boolean;
  status: (WindowsPrinterStatusCode | string)[];
  jobs: number;
  dpi_x: number | null;
  dpi_y: number | null;
  paper_mm: { width: number; height: number } | null;
  custom_paper_supported: boolean;
  kind_hint: AgentKind;
}

export interface AgentDiagnosticCheck {
  id: string;
  title: string;
  status: DiagnosticStatus;
  detail: string;
  hint: string;
}

export type AgentJobStatus = 'queued' | 'sent' | 'done' | 'failed' | 'expired';

export interface AgentJob {
  id: number;
  status: AgentJobStatus;
  kind: 'raw' | 'bitmap' | 'pdf' | string;
  windows_printer: string;
  printer_name: string;
  title: string;
  error: string;
  created_at: string;
  finished_at: string | null;
  duration_ms: number | null;
}

export interface PrintAgentDetail extends PrintAgent {
  printers: AgentWindowsPrinter[];
  diagnostics: { checks: AgentDiagnosticCheck[]; checked_at: string | null } | null;
  stats: {
    ok_last_hour: number;
    failed_last_hour: number;
    last_error: string | null;
    last_error_at: string | null;
  } | null;
  jobs: AgentJob[];
}
