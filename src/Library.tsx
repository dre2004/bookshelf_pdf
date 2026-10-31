import { useState } from "react";
import { coverUrl } from "./api";
import type { BookSummary } from "./types";

function Cover({ book, className }: { book: BookSummary; className?: string }) {
  const [failed, setFailed] = useState(false);
  if (!book.hasCover || failed) {
    return (
      <div className={`cover-fallback ${className ?? ""}`}>
        <span>{book.title}</span>
      </div>
    );
  }
  return (
    <img
      className={className}
      src={coverUrl(book.bookId, book.lastOpenedAt)}
      alt=""
      draggable={false}
      onError={() => setFailed(true)}
    />
  );
}

function ProgressBar({ pct }: { pct: number }) {
  return (
    <div className="progress-track" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
      <div className="progress-fill" style={{ width: `${pct}%` }} />
    </div>
  );
}

interface Props {
  books: BookSummary[];
  onOpenBook: (book: BookSummary) => void;
  onOpenFile: () => void;
  onRemoveBook: (book: BookSummary) => void;
}

export default function Library({ books, onOpenBook, onOpenFile, onRemoveBook }: Props) {
  const started = books.filter((b) => b.lastPage > 0);
  const continueBook = started[0] ?? null;

  return (
    <main className="library">
      <header className="library-header">
        <h1>bookshelf_pdf</h1>
        <button className="btn primary" onClick={onOpenFile}>
          Open PDF
        </button>
      </header>

      {books.length === 0 ? (
        <div className="library-empty">
          <h2>Your shelf is empty</h2>
          <p className="muted">Open a PDF and it will live here — with covers, progress and your reading position.</p>
        </div>
      ) : (
        <>
          {continueBook && (
            <section className="continue-section">
              <span className="section-label">Continue reading</span>
              <div
                className="continue-card"
                role="button"
                tabIndex={0}
                onClick={() => onOpenBook(continueBook)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") onOpenBook(continueBook);
                }}
              >
                <Cover book={continueBook} className="continue-cover" />
                <div className="continue-info">
                  <h2>{continueBook.title}</h2>
                  <span className="muted">
                    page {continueBook.lastPage + 1} of {continueBook.pageCount}
                  </span>
                  <ProgressBar pct={continueBook.progressPct} />
                </div>
              </div>
            </section>
          )}

          <section className="shelf-section">
            <span className="section-label">Library</span>
            <div className="shelf">
              {books.map((book) => (
                <div key={book.bookId} className="shelf-item">
                  <button className="shelf-cover-btn" onClick={() => onOpenBook(book)} title={book.path}>
                    <Cover book={book} className="shelf-cover" />
                    {book.lastPage > 0 && (
                      <span className="shelf-progress">{book.progressPct}%</span>
                    )}
                  </button>
                  <div className="shelf-meta">
                    <span className="shelf-title" title={book.title}>
                      {book.title}
                    </span>
                    <ProgressBar pct={book.progressPct} />
                  </div>
                  <button
                    className="shelf-remove"
                    onClick={() => onRemoveBook(book)}
                    title="Remove from library (keeps the file)"
                    aria-label={`Remove ${book.title} from library`}
                  >
                    ✕
                  </button>
                </div>
              ))}
            </div>
          </section>
        </>
      )}
    </main>
  );
}
