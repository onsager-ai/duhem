// #436: the vertical splitter between the run report's rail and detail
// pane. Pointer-drag and keyboard resize the rail; the width itself is
// owned by `useRailWidth` (RunScaffold.tsx) and lands on one CSS custom
// property on the grid root. Hidden at the `md:` breakpoint — below it
// the rail and detail pane stack instead of sitting side by side, so
// there is nothing to resize (the same breakpoint `.run-results-rail`'s
// `max-height: 42vh` mobile rule already gates in styles.css).

import { useCallback, useEffect, useRef } from "react";

import { cn } from "@/lib/utils";
import { RAIL_WIDTH_STEP } from "../hooks/use-rail-width";

export function RailSplitter({
  width,
  min,
  max,
  onChange,
  onReset,
}: {
  width: number;
  min: number;
  max: number;
  onChange: (next: number) => void;
  onReset: () => void;
}) {
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const handlePointerMove = useCallback(
    (event: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      onChange(drag.startWidth + (event.clientX - drag.startX));
    },
    [onChange],
  );

  const stopDrag = useCallback(() => {
    dragRef.current = null;
    window.removeEventListener("pointermove", handlePointerMove);
    window.removeEventListener("pointerup", stopDrag);
    document.body.style.removeProperty("cursor");
    document.body.style.removeProperty("user-select");
  }, [handlePointerMove]);

  // Drag listeners live on `window`, not the splitter node, so a fast
  // drag that outruns the 2px hit target keeps tracking the pointer —
  // clean up on unmount in case a drag is still in flight.
  useEffect(() => stopDrag, [stopDrag]);

  const startDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    dragRef.current = { startX: event.clientX, startWidth: width };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", stopDrag, { once: true });
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    switch (event.key) {
      case "ArrowLeft":
        event.preventDefault();
        onChange(width - RAIL_WIDTH_STEP);
        break;
      case "ArrowRight":
        event.preventDefault();
        onChange(width + RAIL_WIDTH_STEP);
        break;
      case "Home":
        event.preventDefault();
        onChange(min);
        break;
      case "End":
        event.preventDefault();
        onChange(max);
        break;
      default:
        break;
    }
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the step list"
      aria-valuenow={Math.round(width)}
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      tabIndex={0}
      data-testid="rail-splitter"
      onPointerDown={startDrag}
      onKeyDown={onKeyDown}
      onDoubleClick={onReset}
      className={cn(
        "group hidden w-3 shrink-0 cursor-col-resize touch-none items-stretch",
        "justify-center outline-none md:flex",
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          "w-px rounded-full bg-border transition-colors",
          "group-hover:bg-accent-foreground/40 group-focus-visible:bg-ring",
        )}
      />
    </div>
  );
}
