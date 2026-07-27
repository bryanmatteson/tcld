# Synthetic Tcl program used to exercise nested bytecode, source-map, debugger,
# and hybrid-runtime behavior. This fixture is intentionally authored for tcld
# and may be redistributed under the repository's Apache-2.0 license.

package provide tcld_fixture 1.0

namespace eval fixture {
    variable baseDir /tmp/tcld-fixture
    variable debug_types {parse runtime io}
    variable initialized 0
    variable debug_level
    array set debug_level {}
}

proc fixture::setValue {name value} {
    variable values
    set values($name) $value
    return $value
}

proc fixture::startlog {path} {
    variable baseDir
    global fixture_startlog
    set err ""
    if {[catch {
        set fixture_startlog [file join $baseDir $path]
    } err]} {
        return -code error $err
    }
    return $fixture_startlog
}

proc fixture::logLevelInit {} {
    variable debug_types
    variable debug_level
    variable initialized

    foreach el $debug_types {
        set tvar "FIXTURE_LOG_LEVEL_$el"
        if {[info exists ::env($tvar)]} {
            set debug_level($el) $::env($tvar)
        } else {
            set debug_level($el) 0
        }
        fixture::setValue "debug_level_$el" $debug_level($el)
    }
    set initialized 1
    return $initialized
}

proc fixture::isValidContent {value} {
    expr {[string length $value] > 0}
}

proc fixture::calculateInterval {value unit} {
    if {![regexp {^([0-9]+)([smhd])$} $value -> amount sourceUnit]} {
        return -code error "invalid interval: $value"
    }
    switch -- $sourceUnit {
        s { set sourceSeconds 1 }
        m { set sourceSeconds 60 }
        h { set sourceSeconds 3600 }
        d { set sourceSeconds 86400 }
    }
    switch -- $unit {
        s { set targetSeconds 1 }
        m { set targetSeconds 60 }
        h { set targetSeconds 3600 }
        d { set targetSeconds 86400 }
        default { return -code error "invalid target unit: $unit" }
    }
    expr {$amount * $sourceSeconds / $targetSeconds}
}

proc fixture::getProperTimeShow {totalSeconds} {
    set minutes [expr {int($totalSeconds / 60)}]
    set seconds [expr {$totalSeconds % 60}]
    return " $minutes minute(s) $seconds second(s)"
}

proc fixture::summarize {records} {
    set totals {}
    foreach record $records {
        lassign $record category amount
        if {![dict exists $totals $category]} {
            dict set totals $category 0
        }
        dict incr totals $category $amount
    }
    return $totals
}

proc fixture::firstEnabled {flags} {
    foreach name [lsort [dict keys $flags]] {
        if {[dict get $flags $name]} {
            return $name
        }
    }
    return ""
}
