import { Link } from "react-router-dom";

import { DuhemMark } from "@/components/brand/DuhemMark";
import { cn } from "@/lib/utils";

// The Duhem lockup: mark + wordmark (docs/duhem-brand.md §1, §6, §7).
// The mark is the real 32-grid geometry in `currentColor` ink with no
// plate behind it; the wordmark is Inter at weight 500 (§6: never
// bolder), slightly tight. `asHeading` renders the name as an <h1> so
// exactly one heading with accessible name "Duhem" is visible at any
// viewport — the self-verification VD (verifications/duhem-dashboard)
// asserts it. The mark stays outside the heading and aria-hidden so the
// heading's accessible name is the wordmark alone.
export function BrandMark({
  asHeading = false,
  onClick,
  className,
}: {
  asHeading?: boolean;
  onClick?: () => void;
  className?: string;
}) {
  const wordmark = "text-[1.0625rem] font-medium leading-none tracking-[-0.01em]";
  return (
    <Link
      to="/"
      onClick={onClick}
      className={cn("flex items-center gap-2 text-foreground", className)}
    >
      <DuhemMark className="size-6 shrink-0" />
      {asHeading ? (
        <h1 className={wordmark}>Duhem</h1>
      ) : (
        <span className={wordmark}>Duhem</span>
      )}
    </Link>
  );
}
