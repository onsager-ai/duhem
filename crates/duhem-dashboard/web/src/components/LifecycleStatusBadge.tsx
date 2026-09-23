import { Badge } from "@/components/ui/badge";
import type { LifecycleBlock } from "../api";

export function LifecycleStatusBadge({ status }: { status: LifecycleBlock["status"] }) {
  return <Badge variant={status === "passed" ? "pass" : status === "failed" ? "fail" : "inconclusive"}>{status}</Badge>;
}
