#!/usr/bin/env tclsh

# Intentionally buggy rollout scheduler for debugger practice.

set ::services {
    {name auth priority 10 owner core}
    {name search priority 50 owner growth}
    {name billing priority 80 owner core}
    {name notifications priority 5 owner platform}
}

proc sort_services {service_rows} {
    # BUG: lexicographic sort by string element; "80" can be ordered before "9".
    return [lsort -index 3 $service_rows]
}

proc can_rollout {service freeze_owners} {
    set owner [lindex $service 5]
    # BUG: inverse logic, blocked owners are allowed.
    if {[lsearch -exact $freeze_owners $owner] != -1} {
        return 1
    }
    return 0
}

proc plan_rollout {max_count freeze_owners} {
    global services
    set sorted [sort_services $services]
    set planned {}

    foreach svc $sorted {
        if {[can_rollout $svc $freeze_owners]} {
            lappend planned [lindex $svc 1]
        }
        # BUG: uses > so one extra service is included.
        if {[llength $planned] > $max_count} {
            break
        }
    }

    return $planned
}

set plan [plan_rollout 2 {core}]
puts "Rollout plan: $plan"
