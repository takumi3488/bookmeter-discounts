This application is breakable.

## Run modes

- `RUN_DAILY_AT_UTC` unset: run once and exit
- `RUN_DAILY_AT_UTC=HH:MM`: stay resident and run once a day at that UTC time, serially. Startup and every run (success or failure) wait for the next future slot; a slot missed by a long run or a restart is skipped. SIGTERM stops the wait or the in-flight run and exits without starting a new run
