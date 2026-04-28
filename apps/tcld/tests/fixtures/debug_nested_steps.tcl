proc inner {} {
    set inner_value 41
    set answer [expr {$inner_value + 1}]
    return $answer
}
proc outer {} {
    set outer_value ready
    inner
    set result 42
    return [list $outer_value $result]
}
outer
