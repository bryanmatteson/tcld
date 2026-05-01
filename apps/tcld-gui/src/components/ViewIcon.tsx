"use client";

import {
  AlertTriangle,
  Circle,
  CircleDot,
  Command,
  Cpu,
  Eye,
  FileCode2,
  FileText,
  FolderOpen,
  Layers,
  List,
  Route,
  Sigma,
  SquareStack,
  Terminal,
  Variable,
  Workflow,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { VIEW_ICON_ROLES, type ViewIconRole } from "@/lib/view-registry";
import type { ViewId } from "@/types";

const VIEW_ROLE_ICONS: Record<ViewIconRole, LucideIcon> = {
  cpu: Cpu,
  list: List,
  file: FileCode2,
  "source-map": FileText,
  compare: Cpu,
  alert: AlertTriangle,
  trace: Route,
  project: FolderOpen,
  inspector: Eye,
  exception: Zap,
  flow: Workflow,
  summary: Sigma,
  variables: Variable,
  stack: Layers,
  console: Terminal,
  breakpoint: CircleDot,
  watchpoint: Circle,
  command: Command,
};

interface ViewIconProps {
  viewId?: ViewId;
  role?: ViewIconRole;
  size?: number;
  className?: string;
}

export function ViewIcon({ viewId, role, size = 13, className }: ViewIconProps) {
  const resolvedRole = role ?? (viewId ? VIEW_ICON_ROLES[viewId] : undefined);
  const Icon = resolvedRole ? VIEW_ROLE_ICONS[resolvedRole] : SquareStack;
  return <Icon size={size} strokeWidth={1.75} className={className} aria-hidden />;
}
