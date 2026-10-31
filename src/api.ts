import { invoke } from "@tauri-apps/api/core";
import type { ApiError, BookState, BookSummary, DocumentInfo, PageInfo } from "./types";

export async function openDocument(path: string, password?: string): Promise<DocumentInfo> {
  return invoke<DocumentInfo>("open_document", { path, password: password ?? null });
}

export async function closeDocument(docId: number): Promise<void> {
  return invoke("close_document", { docId });
}

export async function getPageDims(docId: number, from: number, to: number): Promise<PageInfo[]> {
  return invoke<PageInfo[]>("get_page_dims", { docId, from, to });
}

export function toApiError(e: unknown): ApiError | null {
  if (typeof e === "object" && e !== null && "code" in e) {
    const candidate = e as { code: unknown };
    if (typeof candidate.code === "string") return e as ApiError;
  }
  return null;
}

// WebView2 maps custom URI schemes onto http subdomains, other platforms use
// the scheme directly — mirror convertFileSrc's platform detection.
const schemeBase = navigator.userAgent.includes("Windows")
  ? "http://pdfimg.localhost"
  : "pdfimg://localhost";

export function pageImageUrl(docId: number, pageIndex: number, widthPx: number): string {
  return `${schemeBase}/page/${docId}/${pageIndex}/${Math.round(widthPx)}`;
}

/** Cover URL is versioned by last-open time so refreshed covers bust the cache. */
export function coverUrl(bookId: number, version: number): string {
  return `${schemeBase}/cover/${bookId}?v=${version}`;
}

export async function bookRegister(
  path: string,
  title: string,
  pageCount: number,
  docId: number,
): Promise<number> {
  return invoke<number>("book_register", { path, title, pageCount, docId });
}

export async function bookState(bookId: number): Promise<BookState | null> {
  return invoke<BookState | null>("book_state", { bookId });
}

export async function bookProgressSave(
  bookId: number,
  page: number,
  offset: number,
): Promise<void> {
  return invoke("book_progress_save", { bookId, page, offset });
}

export async function libraryList(): Promise<BookSummary[]> {
  return invoke<BookSummary[]>("library_list");
}

export async function libraryRemove(bookId: number): Promise<void> {
  return invoke("library_remove", { bookId });
}

export async function settingsAll(): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("settings_all");
}

export async function settingsSet(key: string, value: string): Promise<void> {
  return invoke("settings_set", { key, value });
}
