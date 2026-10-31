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

export type ReadModeString = ReadMode;
export type ThemeString = Theme;
