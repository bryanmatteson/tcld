// Metadata for source examples embedded by load_example.
// The backend compiles current source through the same path used for edits.
export interface TcldExample {
  id: string;
  filename: string;
  title: string;
  description: string;
  tags: string[];
}

export const EXAMPLES: TcldExample[] = [
  { id: "hello", filename: "hello.tcl", title: "Hello, world", description: "Sets and reads one variable.", tags: ["beginner", "variables"] },
  { id: "proc", filename: "proc.tcl", title: "Procedures", description: "Defines two procedures and calls both from a list.", tags: ["beginner", "procs"] },
  { id: "for-break", filename: "break.tcl", title: "Nested loops with break", description: "Uses break in nested loops to produce exception ranges.", tags: ["control flow", "loops", "exceptions"] },
  { id: "expr", filename: "expr.tcl", title: "Expressions", description: "Inspects nested arithmetic expressions and their exception ranges.", tags: ["expressions", "arithmetic"] },
  { id: "foreach", filename: "foreach.tcl", title: "foreach", description: "Builds and traverses lists with nested loops.", tags: ["loops", "collections"] },
  { id: "catch", filename: "catch.tcl", title: "catch / errors", description: "Catches an error and reads the resulting value.", tags: ["exceptions", "error handling"] },
];

export function getExampleById(id: string): TcldExample | undefined {
  return EXAMPLES.find((example) => example.id === id);
}
