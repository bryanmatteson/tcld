// ─── Bundled example Tcl programs ─────────────────────────────
//
// Examples are loaded through `store.loadText(source, label)` and the
// `load_text` Tauri command. Keeping them inline avoids a separate resource
// bundle.

export interface TcldExample {
  id: string;
  filename: string;
  title: string;
  description: string;
  tags: string[];
  source: string;
}

/**
 * Each `source` block mirrors a fixture from `apps/tcld/tests/fixtures/`
 * (the canonical regression set). Inline sources use the same load path in
 * development and packaged builds.
 */
export const EXAMPLES: TcldExample[] = [
  {
    id: "hello",
    filename: "hello.tcl",
    title: "Hello, world",
    description: "Sets and reads one variable.",
    tags: ["beginner", "variables"],
    source: `# hello.tcl --
# Baseline opcode shape for a small program.

set msg "hello, world"
set msg
`,
  },
  {
    id: "proc",
    filename: "proc.tcl",
    title: "Procedures",
    description: "Defines two procedures and calls both from a list.",
    tags: ["beginner", "procs"],
    source: `# proc.tcl --
# Two procedures called from a list.

proc a { x } {
    return "$x : $x"
}

proc b { x } {
    return "$x > $x"
}

list [a TEST] [b TEST]
`,
  },
  {
    id: "for-break",
    filename: "break.tcl",
    title: "Nested loops with break",
    description: "Uses break in nested loops to produce exception ranges.",
    tags: ["control flow", "loops", "exceptions"],
    source: `# break.tcl --
# Nested loops with break statements and exception ranges.

set result {}
set top 8

for {set outerCounter 1} {1} {incr outerCounter} {
    if {$outerCounter > $top} {
        break
    }
    append result $outerCounter

    for {set innerCounter 1} {1} {incr innerCounter} {
        if {$innerCounter > $outerCounter} {
            break
        }
        append result .
    }
}

append result DONE
set result
`,
  },
  {
    id: "expr",
    filename: "expr.tcl",
    title: "Expressions",
    description: "Compares arithmetic and relational expression bytecode.",
    tags: ["expressions", "arithmetic"],
    source: `# expr.tcl --
# Expressions in one context for comparing compiler output.

set a 21
set b 2
set sum [expr {$a + $b}]
set product [expr {$a * $b}]
set check [expr {$sum > $product}]
list $sum $product $check
`,
  },
  {
    id: "foreach",
    filename: "foreach.tcl",
    title: "foreach",
    description: "Traverses a list with foreach.",
    tags: ["loops", "collections"],
    source: `# foreach.tcl --
# Walking a list with foreach.

set words {one two three four five}
set acc {}
foreach w $words {
    lappend acc [string length $w]
}
set acc
`,
  },
  {
    id: "catch",
    filename: "catch.tcl",
    title: "catch / errors",
    description: "Handles procedure errors with catch.",
    tags: ["exceptions", "error handling"],
    source: `# catch.tcl --
# Catch around a call that may return an error.

proc risky {n} {
    if {$n < 0} { error "negative input: $n" }
    return [expr {$n * $n}]
}

set out {}
foreach n {3 0 -1 7} {
    if {[catch {risky $n} result]} {
        lappend out "err: $result"
    } else {
        lappend out "ok: $result"
    }
}
set out
`,
  },
];

export function getExampleById(id: string): TcldExample | undefined {
  return EXAMPLES.find((example) => example.id === id);
}
