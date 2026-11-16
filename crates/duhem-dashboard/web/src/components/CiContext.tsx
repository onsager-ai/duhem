// Shared run-header and list metadata. Bundle URLs are producer-supplied.
export function CiContext({
  branch,
  pr,
  ci_url,
}: {
  branch?: string;
  pr?: string;
  ci_url?: string;
}) {
  const href = ci_url && /^https?:\/\//i.test(ci_url) ? ci_url : undefined;
  if (!branch && !pr && !href) return null;
  return (
    <span className="inline-flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
      {branch && (
        <span className="max-w-[24ch] truncate" title={branch}>
          {branch}
        </span>
      )}
      {pr && <span>PR #{pr}</span>}
      {href && (
        <a href={href} target="_blank" rel="noreferrer" className="hover:underline">
          CI
        </a>
      )}
    </span>
  );
}
