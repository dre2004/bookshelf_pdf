export interface PageInfo {
  /** Width in PDF points. */
  width: number;
  /** Height in PDF points. */
  height: number;
}

export interface DocumentInfo {
  docId: number;
  pageCount: number;
  /** Dimensions for the first pages; the rest arrive via getPageDims. */
  pages: PageInfo[];
}

export type ReadMode = "paged" | "scroll";

/** `percent` is relative to fit-width (100 = page fits the viewport width). */
export type ZoomMode =
  | { kind: "fitWidth" }
  | { kind: "fitPage" }
  | { kind: "percent"; value: number };

export type Theme = "day" | "night" | "sepia";

/** Shape of command errors serialized by the Rust side (`{ code, ... }`). */
export interface ApiError {
  code: string;
  reason?: string;
}

export interface BookSummary {
  bookId: number;
  path: string;
  title: string;
  pageCount: number;
  lastPage: number;
  /** Position within the last page, 0..1. */
  lastOffset: number;
  progressPct: number;
  lastOpenedAt: number;
  hasCover: boolean;
}

export interface BookState {
  lastPage: number;
  lastOffset: number;
}

/** Character-level text geometry for one page (see Rust PageTextData). */
export interface PageTextData {
  /** One character per entry; `boxes[i*4..i*4+4]` is its box in PDF points. */
  chars: string;
  boxes: number[];
  width: number;
  height: number;
}

export interface SearchMatch {
  page: number;
  /** Character range into PageTextData.chars. */
  start: number;
  end: number;
}

/** One clickable link on a page: a TOC entry, cross reference or URI.
 *  `rect` follows PageTextData's box convention (PDF points, origin at the
 *  page's bottom-left). */
export interface PageLink {
  rect: [number, number, number, number];
  /** Target page index for in-document links. */
  destPage: number | null;
  /** External URI, for links that open one. */
  uri: string | null;
}

export interface OutlineNode {
  title: string;
  /** Target page index, -1 when the outline entry has no page. */
  page: number;
  children: OutlineNode[];
}

export interface Bookmark {
  id: number;
  page: number;
  offset: number;
  title: string;
  createdAt: number;
}

export interface Highlight {
  id: number;
  page: number;
  startChar: number;
  endChar: number;
  text: string;
  color: string;
  note: string;
  createdAt: number;
}

export type HighlightColor = "yellow" | "green" | "blue" | "red";

export const HIGHLIGHT_COLORS: HighlightColor[] = ["yellow", "green", "blue", "red"];

export type ReadModeString = ReadMode;
export type ThemeString = Theme;
