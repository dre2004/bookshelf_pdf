// @vitest-environment jsdom
// Regression test for the blank-window-on-open bug: renders the real App
// with a mocked Tauri backend and clicks a book in the library. If any
// component throws during the reader render, this test fails with the
// actual error instead of the user seeing a white window.
import { describe, expect, it, vi, beforeAll, beforeEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import React from "react";

const pageDims = Array.from({ length: 40 }, () => ({ width: 595, height: 842 }));
const doc = {
  docId: 7,
  pageCount: 40,
  pages: pageDims.slice(0, 200),
};

vi.mock("./api", () => ({
  openDocument: vi.fn().mockResolvedValue(doc),
  closeDocument: vi.fn().mockResolvedValue(undefined),
  get_page_dims: undefined,
  getPageDims: vi.fn().mockResolvedValue(pageDims.slice(0, 40)),
  getPageText: vi.fn().mockResolvedValue({
    chars: "Hello page text",
    boxes: [10, 10, 50, 12],
    width: 595,
    height: 842,
  }),
  cachedPageText: vi.fn().mockResolvedValue({
    chars: "Hello page text",
    boxes: [10, 10, 50, 12],
    width: 595,
    height: 842,
  }),
  pageImageUrl: vi.fn(() => "http://pdfimg.localhost/page/7/0/800"),
  coverUrl: vi.fn(() => "http://pdfimg.localhost/cover/1"),
  bookRegister: vi.fn().mockResolvedValue(1),
  bookState: vi.fn().mockResolvedValue({ lastPage: 2, lastOffset: 0.5 }),
  bookProgressSave: vi.fn().mockResolvedValue(undefined),
  libraryList: vi.fn().mockResolvedValue([
    {
      bookId: 1,
      path: "C:\\books\\test.pdf",
      title: "test.pdf",
      pageCount: 40,
      lastPage: 2,
      lastOffset: 0.5,
      progressPct: 5,
      lastOpenedAt: 1,
      hasCover: true,
    },
  ]),
  libraryRemove: vi.fn().mockResolvedValue(undefined),
  settingsAll: vi.fn().mockResolvedValue({}),
  settingsSet: vi.fn().mockResolvedValue(undefined),
  getStartupFile: vi.fn().mockResolvedValue(null),
  getOutline: vi.fn().mockResolvedValue([]),
  searchDocument: vi.fn().mockResolvedValue([]),
  bookmarksAdd: vi.fn().mockResolvedValue({ id: 1, page: 0, offset: 0, title: "t", createdAt: 0 }),
  bookmarksList: vi.fn().mockResolvedValue([]),
  bookmarksRemove: vi.fn().mockResolvedValue(undefined),
  highlightsAdd: vi.fn().mockResolvedValue({}),
  highlightsList: vi.fn().mockResolvedValue([]),
  highlightsRemove: vi.fn().mockResolvedValue(undefined),
  highlightsUpdate: vi.fn().mockResolvedValue(undefined),
  ttsVoices: vi.fn().mockResolvedValue([]),
  ttsSpeak: vi.fn().mockResolvedValue(undefined),
  ttsPause: vi.fn().mockResolvedValue(undefined),
  ttsResume: vi.fn().mockResolvedValue(undefined),
  ttsStop: vi.fn().mockResolvedValue(undefined),
  ttsInstallDefaultVoices: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
  ask: vi.fn().mockResolvedValue(true),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    setTitle: vi.fn().mockResolvedValue(undefined),
  })),
}));

beforeAll(() => {
  // jsdom lacks the browser APIs Reader relies on.
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  }
  vi.stubGlobal("ResizeObserver", FakeObserver);
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener() {}, removeEventListener() {} })));
});

beforeEach(() => {
  vi.clearAllMocks();
});

describe("App open-book regression", () => {
  it("renders the reader after clicking a library book without crashing", async () => {
    const { default: App } = await import("./App");

    let container: HTMLElement;
    let error: unknown = null;
    // Capture render errors instead of letting them unmount silently.
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await act(async () => {
        const rendered = render(React.createElement(App));
        container = rendered.container;
      });
    } catch (e) {
      error = e;
    }
    expect(error).toBeNull();

    // Library view should be visible.
    await screen.findAllByText("test.pdf");
    const bookButton = container!.querySelector<HTMLElement>(".shelf-cover-btn");
    expect(bookButton).toBeTruthy();

    await act(async () => {
      bookButton!.click();
    });

    // If any component throws post-click, findBy* will time out or the
    // error spy above will have captured it. Assert the reader painted.
    await waitFor(
      () => {
        const frame = container!.querySelector(".page-frame");
        expect(frame).toBeTruthy();
      },
      { timeout: 3000 },
    );
    consoleError.mockRestore();
  });
});
