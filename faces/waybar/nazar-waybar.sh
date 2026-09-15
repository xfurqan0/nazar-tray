#!/bin/sh
# nazar-waybar.sh -- a Waybar custom module for nazar-tray.
#
# Prints one line of JSON: the binding window's percentage for the bar, every window for
# the tooltip, and a class for the stylesheet. It reads ~/.nazar/limits.json and
# ~/.nazar/limits.lock and nothing else -- no file is written, no socket is opened, and no
# program but jq is run. What each rule is for, and every state this can print, is in
# README.md beside this file.
set -u

home=${NAZAR_HOME:-${HOME:-/nonexistent}/.nazar}
limits=$home/limits.json
lock=$home/limits.lock
[ -r "$limits" ] || limits=/dev/null
[ -r "$lock" ] || lock=/dev/null

# The faster half of "is the tray running". A pid the kernel has never heard of is stale at
# once, where the heartbeat takes five minutes to say so; a pid that cannot be asked about
# is left to the heartbeat below, because "cannot tell" is not "nobody is there".
pid=$(jq -r '.pid // empty' "$lock" 2>/dev/null)
gone=false
[ -n "$pid" ] && [ ! -d "/proc/$pid" ] && gone=true

line=$(jq -cn --argjson gone "$gone" --slurpfile doc "$limits" --slurpfile lk "$lock" '
  def at($v): $v // "" | (try fromdateiso8601 catch null);
  def span($s): if $s < 3600 then "\(($s/60)|floor) m"
    elif $s < 86400 then "\(($s/3600)|floor) h \((($s%3600)/60)|floor) m"
    else "\(($s/86400)|floor) d \((($s%86400)/3600)|floor) h" end;
  # Rule 2 of the contract: a window with no percent is unknown, and never a reassuring 0.
  def pct($w): if ($w.percent // null) == null then "?" else "\($w.percent|floor) %" end;
  now as $t
  | ($doc[0].providers // {}) as $ps
  | (($gone | not) and (at($lk[0].heartbeatAt) // 0) > $t - 300) as $up
  # `binding` is a summary the writer computed; one that names a window with no percent is
  # recomputed here, which is what the contract tells a disagreeing consumer to do.
  | ([ $ps | .[] as $p
       | if ($p.binding // null) != null and (($p.windows[$p.binding].percent) // null) != null
         then $p.windows[$p.binding].percent
         else (([$p.windows // {} | .[].percent // empty] | max) // empty) end ]) as $tops
  # Rule 5: rounded down, at display time. 99.6 % is not 100 %.
  | (if ($tops | length) == 0 then null else ($tops | max | floor) end) as $n
  | (if $up | not then "stale" elif $n == null then "unknown"
     elif $n >= 85 then "crit" elif $n >= 60 then "warn" else "ok" end) as $class
  | ([ $ps | to_entries[] | .key as $k | .value as $p
       | if ($p.configured // false) | not then ["\($k): not configured"]
         else [ $p.windows // {} | to_entries[]
                | "\($k) \(.key) \(pct(.value))"
                  + (if at(.value.resetsAt) == null then ""
                     elif at(.value.resetsAt) <= $t then " · reset due"
                     else " · resets in \(span(at(.value.resetsAt) - $t))" end) ]
              + (if at($p.sourceAt) == null then []
                 else ["\($k) read \(span($t - at($p.sourceAt))) ago"] end)
         end ] | flatten) as $rows
  | { text: ("nazar " + (if $n == null then "?" else "\($n)%" end)),
      alt: $class,
      class: $class,
      tooltip: ((if $up then [] else ["nazar-tray is not running"] end) + $rows | join("\r")),
      percentage: ($n // 0) }
' 2>/dev/null)

# Waybar hides a module whose script prints nothing or prints something it cannot parse, so
# the one thing this must never do is fail quietly. Every way of getting here ends in a line.
[ -n "$line" ] || line='{"text":"nazar ?","alt":"unknown","class":"unknown","tooltip":"nazar-tray: limits.json could not be read","percentage":0}'
printf '%s\n' "$line"
