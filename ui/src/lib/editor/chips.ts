// The chip model shared by the assisted-mode surface and the send-result
// surface. A chip is DISPLAY-only: it tells the user what the server will do (or
// did) to the request on send. The server is the source of truth — a chip never
// implies an edit the client applied itself.
//
// Kept intentionally minimal: a label and an optional state string. The state is
// rendered as plain text (never markup), so it is safe for any value.
export interface Chip {
  /** The thing the chip is about, e.g. "Content-Length". */
  label: string;
  /** The chip's qualifier, e.g. "skipped: TE present" / "update" / "off". */
  state?: string;
}
