_2026-06-30_

## English

kmod: hide VPN interfaces from the SIOCGIFCONF buffer-size query (ifc_req == NULL), not just the filled list, so the legacy two-step enumeration can no longer reveal a hidden interface by comparing the advertised count against the returned entries.

## Русский

kmod: скрывать VPN-интерфейсы и при запросе размера буфера SIOCGIFCONF (ifc_req == NULL), а не только в заполненном списке, чтобы устаревшее двухшаговое перечисление не выдавало скрытый интерфейс по расхождению размера и числа записей.
