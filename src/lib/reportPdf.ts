// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//
// Client-side PDF rendering for the AI Report Wizard (v0.4.0). Builds a
// clean, paginated document from the structured report + deterministic
// figures, entirely in the webview. The bytes are persisted to a
// user-chosen path through the `report_save_pdf` command (no broad fs
// capability granted).
import { PDFDocument, StandardFonts, rgb, type PDFFont } from "pdf-lib";

import type { GeneratedReportResponse } from "@/lib/tauri";
import { formatMoney } from "@/lib/format";

const PAGE_W = 595.28; // A4 points
const PAGE_H = 841.89;
const MARGIN = 56;
const CONTENT_W = PAGE_W - MARGIN * 2;
const INK = rgb(0.12, 0.14, 0.16);
const MUTED = rgb(0.42, 0.46, 0.5);
const ACCENT = rgb(0.18, 0.42, 0.27);

function wrap(text: string, font: PDFFont, size: number, maxW: number): string[] {
  const out: string[] = [];
  for (const rawLine of text.split("\n")) {
    const words = rawLine.split(/\s+/).filter(Boolean);
    if (words.length === 0) {
      out.push("");
      continue;
    }
    let line = "";
    for (const w of words) {
      const trial = line ? `${line} ${w}` : w;
      if (font.widthOfTextAtSize(trial, size) <= maxW || !line) {
        line = trial;
      } else {
        out.push(line);
        line = w;
      }
    }
    if (line) out.push(line);
  }
  return out;
}

function uint8ToBase64(bytes: Uint8Array): string {
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

/** Render the report to PDF bytes, base64-encoded for the save command. */
export async function buildReportPdfBase64(
  result: GeneratedReportResponse,
  currency: string,
  locale: string | null,
): Promise<string> {
  const doc = await PDFDocument.create();
  const font = await doc.embedFont(StandardFonts.Helvetica);
  const bold = await doc.embedFont(StandardFonts.HelveticaBold);

  let page = doc.addPage([PAGE_W, PAGE_H]);
  let y = PAGE_H - MARGIN;

  const newPage = () => {
    page = doc.addPage([PAGE_W, PAGE_H]);
    y = PAGE_H - MARGIN;
  };

  const draw = (
    text: string,
    opts: { size?: number; font?: PDFFont; color?: typeof INK; gap?: number } = {},
  ) => {
    const size = opts.size ?? 11;
    const f = opts.font ?? font;
    const color = opts.color ?? INK;
    for (const line of wrap(text, f, size, CONTENT_W)) {
      if (y < MARGIN + size) newPage();
      page.drawText(line, { x: MARGIN, y: y - size, size, font: f, color });
      y -= size * 1.38;
    }
    y -= opts.gap ?? 0;
  };

  const { report, figures } = result;

  draw(`Spending report — ${result.timeframe_label}`, {
    size: 20,
    font: bold,
    gap: 4,
  });
  const meta =
    (result.provider === "ollama"
      ? `Generated locally (${result.model}) · no API cost`
      : `Generated with ${result.model} · API cost ${fmtMicros(result.cost_micros)}`) +
    ` · total spend ${formatMoney(figures.total_spent_cents, currency, locale)} over ${figures.days} days · ${result.generated_at}`;
  draw(meta, { size: 9, color: MUTED, gap: 14 });

  if (report.overall_summary) {
    draw(report.overall_summary, { size: 11, gap: 14 });
  }

  for (const s of report.sections) {
    draw(s.heading, { size: 14, font: bold, color: ACCENT, gap: 2 });
    if (s.summary) draw(s.summary, { size: 11, gap: 2 });
    for (const b of s.bullets) {
      // Hanging indent for bullets.
      for (const [idx, line] of wrap(
        b,
        font,
        11,
        CONTENT_W - 14,
      ).entries()) {
        if (y < MARGIN + 11) newPage();
        const prefix = idx === 0 ? "•  " : "   ";
        page.drawText(prefix + line, {
          x: MARGIN + 6,
          y: y - 11,
          size: 11,
          font,
          color: INK,
        });
        y -= 11 * 1.38;
      }
    }
    y -= 12;
  }

  if (y < MARGIN + 30) newPage();
  y -= 8;
  draw(
    "Figures are computed locally from your data; the narrative is AI-generated and may contain errors — verify before acting.",
    { size: 8, color: MUTED },
  );

  return uint8ToBase64(await doc.save());
}

function fmtMicros(micros: number): string {
  const d = micros / 1_000_000;
  const abs = Math.abs(d);
  if (abs >= 1) return `$${d.toFixed(2)}`;
  if (abs >= 0.01) return `$${d.toFixed(3)}`;
  return `$${d.toFixed(4)}`;
}
