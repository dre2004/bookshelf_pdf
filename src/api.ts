import { invoke } from "@tauri-apps/api/core";
import type {
  ApiError,
  Bookmark,
  BookState,
  BookSummary,
  DocumentInfo,
  Highlight,
  OutlineNode,
  PageTextData,
  PageInfo,
  SearchMatch,
} from "./types";

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

// ---- text, search, outline ----

export async function getPageText(docId: number, page: number): Promise<PageTextData> {
  return invoke<PageTextData>("get_page_text", { docId, page });
}

export async function searchDocument(
  docId: number,
  query: string,
  caseSensitive: boolean,
): Promise<SearchMatch[]> {
  return invoke<SearchMatch[]>("search_document", { docId, query, caseSensitive });
}

export async function getOutline(docId: number): Promise<OutlineNode[]> {
  return invoke<OutlineNode[]>("get_outline", { docId });
}

// ---- bookmarks ----

export async function bookmarksAdd(
  bookId: number,
  page: number,
  offset: number,
  title: string,
): Promise<Bookmark> {
  return invoke<Bookmark>("bookmarks_add", { bookId, page, offset, title });
}

export async function bookmarksRemove(bookmarkId: number): Promise<void> {
  return invoke("bookmarks_remove", { bookmarkId });
}

export async function bookmarksList(bookId: number): Promise<Bookmark[]> {
  return invoke<Bookmark[]>("bookmarks_list", { bookId });
}

// ---- highlights ----

export async function highlightsAdd(entry: {
  bookId: number;
  page: number;
  startChar: number;
  endChar: number;
  text: string;
  color: string;
  note: string;
}): Promise<Highlight> {
  return invoke<Highlight>("highlights_add", { entry });
}

export async function highlightsUpdate(
  highlightId: number,
  color?: string,
  note?: string,
): Promise<void> {
  return invoke("highlights_update", {
    highlightId,
    color: color ?? null,
    note: note ?? null,
  });
}

export async function highlightsRemove(highlightId: number): Promise<void> {
  return invoke("highlights_remove", { highlightId });
}

export async function highlightsList(bookId: number): Promise<Highlight[]> {
  return invoke<Highlight[]>("highlights_list", { bookId });
}

// ---- text data cache (shared by text layer, search and highlight overlays) ----

const textCache = new Map<string, Promise<PageTextData>>();
const TEXT_CACHE_MAX = 80;

export function cachedPageText(docId: number, page: number): Promise<PageTextData> {
  const key = `${docId}:${page}`;
  let entry = textCache.get(key);
  if (!entry) {
    entry = getPageText(docId, page).catch((e) => {
      textCache.delete(key);
      throw e;
    });
    textCache.set(key, entry);
    if (textCache.size > TEXT_CACHE_MAX) {
      const oldest = textCache.keys().next().value;
      if (oldest !== undefined) textCache.delete(oldest);
    }
  }
  return entry;
}
