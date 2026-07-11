import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "./i18n";
import { PdfViewer } from "./components/PdfViewer";
// side-effect import: テーマ（light/dark/accent）を適用し、メインウィンドウでの変更を
// storage イベントで同期する（CR-037: 以前は別ウィンドウがテーマ未適用だった）。
import "./hooks/useTheme";
import "./index.css";

const params = new URLSearchParams(window.location.search);
const idStr = params.get("id");
const id = idStr ? Number(idStr) : NaN;
const pageStr = params.get("page");
const initialPage = pageStr ? Number(pageStr) : undefined;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {Number.isFinite(id) ? (
      <PdfViewer attachmentId={id} initialPage={initialPage} />
    ) : (
      <div style={{ padding: 24, fontFamily: "system-ui", color: "#888" }}>
        {i18n.t("pdfViewer.noAttachmentId")}
      </div>
    )}
  </React.StrictMode>,
);
