#!/usr/bin/env tclsh

# Intentionally buggy contact book for debugger practice.

set ::contacts [dict create]

proc add_contact {name phone} {
    global contacts
    dict set contacts $name $phone
}

proc update_contact {name new_phone} {
    # BUG: missing global contacts, updates only local variable.
    set contacts [dict replace $contacts $name $new_phone]
}

proc find_contact {name} {
    global contacts
    if {[dict exists $contacts $name]} {
        return [dict get $contacts $name]
    }
    return "not found"
}

proc export_contacts {} {
    global contacts
    puts "=== Contacts ==="
    foreach name [dict keys $contacts] {
        # BUG: prints all names but always reads "alice".
        puts "$name -> [dict get $contacts alice]"
    }
}

proc main {} {
    add_contact alice 555-1000
    add_contact bob 555-2222
    add_contact carol 555-3333

    puts "Before update: [find_contact bob]"
    update_contact bob 555-9999
    puts "After update: [find_contact bob]"
    export_contacts
}

main
