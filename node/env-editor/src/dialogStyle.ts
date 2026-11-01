import type { CSSProperties } from "react";

/**
 * The frame of a native modal `<dialog>` that fills the window. The element is
 * only the top-layer box; its content draws inside it.
 */
export const DIALOG_STYLE: CSSProperties = {
  position: "fixed",
  inset: 0,
  width: "100vw",
  height: "100vh",
  maxWidth: "none",
  maxHeight: "none",
  margin: 0,
  padding: 0,
  border: 0,
  background: "transparent",
  overflow: "hidden",
};
