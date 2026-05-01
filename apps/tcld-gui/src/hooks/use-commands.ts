"use client";

import { useEffect, useMemo, useRef } from "react";
import { commandRegistry, type CommandDef } from "@/lib/command-registry";

function buildCommandShapeKey(commands: CommandDef[]): string {
  return commands
    .map((command) => [
      command.id,
      command.title,
      command.category,
      command.when ?? "",
      command.allowInInput ? "1" : "0",
      (command.shortcut ?? []).join(","),
    ].join("|"))
    .join("\n");
}

export function useCommands(commands: CommandDef[]) {
  const commandsRef = useRef(commands);
  const runsByIdRef = useRef(new Map<string, () => void>());
  const shapeKey = useMemo(() => buildCommandShapeKey(commands), [commands]);

  useEffect(() => {
    commandsRef.current = commands;
    const next = new Map<string, () => void>();
    for (const command of commands) {
      next.set(command.id, command.run);
    }
    runsByIdRef.current = next;
  }, [commands]);

  useEffect(() => {
    const snapshot = commandsRef.current;
    const unregister = snapshot.map((command) => commandRegistry.register({
      ...command,
      // Keep callbacks fresh without re-registering on every render.
      run: () => {
        const run = runsByIdRef.current.get(command.id);
        if (run) run();
      },
    }));

    return () => {
      for (const fn of unregister) fn();
    };
  }, [shapeKey]);
}
