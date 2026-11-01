import { useCallback, useEffect, useRef } from "react";

/**
 * Drag handle between the sidebar and the reader. The sidebar starts at the
 * window's left edge, so the pointer's viewport x is the sidebar width;
 * the parent owns clamping and persistence.
 */
export default function Resizer({ onResize }: { onResize: (width: number) => void }) {
  const dragging = useRef(false);

  const move = useCallback(
    (event: MouseEvent) => {
      if (dragging.current) onResize(event.clientX);
    },
    [onResize],
  );

  useEffect(() => {
    const up = () => {
      dragging.current = false;
      document.body.classList.remove("resizing");
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [move]);

  return (
    <div
      className="sidebar-resizer"
      onMouseDown={(event) => {
        event.preventDefault();
        dragging.current = true;
        document.body.classList.add("resizing");
      }}
      role="separator"
      aria-orientation="vertical"
      title="Drag to resize"
    />
  );
}
