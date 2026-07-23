import { Link } from "react-router-dom";
import { SquareCheckBig } from "lucide-react";

import { cn } from "@/lib/utils";

// The Duhem wordmark. `asHeading` renders the name as an <h1> so exactly
// one heading with accessible name "Duhem" is visible at any viewport —
// the self-verification VD (verifications/duhem-dashboard) asserts it.
export function BrandMark({
  asHeading = false,
  onClick,
  className,
}: {
  asHeading?: boolean;
  onClick?: () => void;
  className?: string;
}) {
  return (
    <Link
      to="/"
      onClick={onClick}
      className={cn("flex items-center gap-2.5", className)}
    >
      <span
        aria-hidden
        className="grid size-7 shrink-0 place-items-center rounded-md bg-primary text-primary-foreground shadow-sm"
      >
        <SquareCheckBig className="size-4" />
      </span>
      {asHeading ? (
        <h1 className="text-[0.95rem] font-semibold tracking-tight">Duhem</h1>
      ) : (
        <span className="text-[0.95rem] font-semibold tracking-tight">
          Duhem
        </span>
      )}
    </Link>
  );
}
