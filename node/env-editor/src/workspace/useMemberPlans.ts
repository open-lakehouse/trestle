// Plan every member of the workspace on its slot, in the background, so the
// shell can show each one's status and catch members that would collide on a
// host port — including the ones not open in a tab.

import { useEffect, useState } from "react";
import { usePlanner } from "../planner";
import type { CatalogDto, PlanResult } from "../types";
import { contextFor, type WorkspaceDoc } from "./model";

/** One member's latest plan, or why it doesn't plan. */
export interface MemberPlan {
  plan?: PlanResult;
  error?: string;
}

/** How long workspace edits settle before the members replan. */
const DEBOUNCE_MS = 300;

/** Each member's plan under its workspace context, keyed by env id. */
export function useMemberPlans(
  workspace: WorkspaceDoc | undefined,
  catalog: CatalogDto | undefined,
): Record<string, MemberPlan> {
  const planner = usePlanner();
  const [plans, setPlans] = useState<Record<string, MemberPlan>>({});

  useEffect(() => {
    if (!workspace || !catalog) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      void Promise.all(
        workspace.envs.map(async (env): Promise<[string, MemberPlan]> => {
          try {
            const plan = await planner.plan(
              env.selection ?? catalog.default_selection,
              undefined,
              contextFor(workspace, env),
            );
            return [env.id, { plan }];
          } catch (e) {
            return [
              env.id,
              { error: e instanceof Error ? e.message : String(e) },
            ];
          }
        }),
      ).then((entries) => {
        if (!cancelled) setPlans(Object.fromEntries(entries));
      });
    }, DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [planner, workspace, catalog]);

  return plans;
}

/** Where a host-side browser reaches `plan`'s gateway, or `undefined` without
 *  one. Mirrors `Plan::gateway_url`: `https://<env>.localhost` under sign-in. */
export function gatewayUrl(
  plan: PlanResult | undefined,
  envName: string,
): string | undefined {
  const port = plan?.gateway.listeners[0]?.host_port;
  if (port === undefined) return undefined;
  return plan?.gateway.auth
    ? `https://${envName}.localhost:${port}`
    : `http://localhost:${port}`;
}
