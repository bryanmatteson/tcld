#!/usr/bin/env tclsh

# Intentionally buggy log monitor for debugger practice.

set ::sample_logs {
    "2026-02-10 INFO Service started"
    "2026-02-10 WARN Slow query"
    "2026-02-10 ERROR Payment failure"
    "2026-02-10 INFO Health check ok"
    "2026-02-10 error disk almost full"
}

proc parse_line {line} {
    # BUG: pattern allows only uppercase levels, lower-case "error" is dropped.
    if {![regexp {^(\S+)\s+(INFO|WARN|ERROR)\s+(.+)$} $line -> date level msg]} {
        return ""
    }
    return [dict create date $date level $level message $msg]
}

proc count_by_level {entries} {
    set counts [dict create INFO 0 WARN 0 ERROR 0]
    foreach entry $entries {
        if {$entry eq ""} {
            continue
        }
        set level [dict get $entry level]
        dict set counts $level [expr {[dict get $counts $level] + 1}]
    }
    return $counts
}

proc latest_error_message {entries} {
    set message "none"
    foreach entry $entries {
        if {$entry eq ""} {
            continue
        }
        # BUG: typo in key name causes runtime error.
        if {[dict get $entry lavel] eq "ERROR"} {
            set message [dict get $entry message]
        }
    }
    return $message
}

proc main {} {
    global sample_logs

    set entries {}
    foreach line $sample_logs {
        lappend entries [parse_line $line]
    }

    set counts [count_by_level $entries]
    puts "INFO:  [dict get $counts INFO]"
    puts "WARN:  [dict get $counts WARN]"
    puts "ERROR: [dict get $counts ERROR]"
    puts "Latest error: [latest_error_message $entries]"
}

main
