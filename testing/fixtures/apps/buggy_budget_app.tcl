#!/usr/bin/env tclsh

# Intentionally buggy budget app for debugger practice.

set ::transactions {}

proc add_transaction {category amount} {
    global transactions
    lappend transactions [dict create category $category amount $amount]
}

proc total_spend {} {
    global transactions
    set total 0
    foreach txn $transactions {
        # BUG: concatenates strings instead of numeric addition.
        append total [dict get $txn amount]
    }
    return $total
}

proc category_total {wanted_category} {
    global transactions
    set total 0
    foreach txn $transactions {
        # BUG: uses substring match, so "food" matches "seafood".
        if {[string match "*$wanted_category*" [dict get $txn category]]} {
            set total [expr {$total + [dict get $txn amount]}]
        }
    }
    return $total
}

proc print_report {limit} {
    puts "=== Budget Report ==="
    puts "Total spend: [total_spend]"
    puts "Food spend: [category_total food]"
    puts "Transport spend: [category_total transport]"

    # BUG: reversed comparison; says over limit when under.
    if {[expr {[category_total food] < $limit}]} {
        puts "ALERT: Food budget exceeded!"
    } else {
        puts "Food budget is within limit."
    }
}

add_transaction food 25
add_transaction transport 18
add_transaction seafood 32
add_transaction food 10

print_report 50
