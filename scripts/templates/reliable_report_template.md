# Push Reliability Report for {{ date }}

Data since {{ meta.since }}

## Totals

* Total Messages processed: *{{ meta.total_count }}*
* Total Messages delivered: *{{ meta.success_count }}*
* Total Messages expired before delivery: *{{ meta.expired_count }}*
* Total Messages errored before delivery (including expired): *{{ meta.fail_count }}*

* Shortest delivery time (seconds): *{{ meta.shortest }}*
* Longest delivery time (seconds): *{{ meta.longest }}*

===

## Milestones

|      milestone       |   count  | total time | average time |
|----------------------|----------|------------|--------------|
{% for milestone,info in meta.milestones.items() -%}
| {{"%20s"| format(milestone)}} | {{"%8d"| format(info.count)}} | {{"%6.6f"| format(info.total_time)}} | {{"%6.6f"| format(info.average_time)}} |
{% endfor %}

