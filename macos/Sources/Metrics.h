#pragma once
#include <stdint.h>
#define MONITOR_MAX_CORES 128
typedef struct {
    double cpu_user, cpu_system, cores[MONITOR_MAX_CORES];
    int core_count, cpu_available;
    uint64_t memory_total, memory_used, memory_cached, memory_wired, memory_compressed;
    uint64_t swap_used, swap_total;
    int memory_available, swap_available;
    double gpu_usage;
    int gpu_available;
    double download, upload, disk_read, disk_write, uptime;
    int network_available, disk_io_available;
    char interfaces[96];
    uint32_t link_mbps;
    double cpu_power_watts, cpu_temperature_c, cpu_frequency_mhz;
    uint32_t cpu_sensor_flags;
} MonitorSample;
typedef struct { int pid; double cpu; uint64_t memory, identity; char name[128]; } MonitorProcess;
typedef struct { uint64_t used, total; char name[128]; } MonitorVolume;
void monitor_sample(MonitorSample *sample);
int monitor_processes(MonitorProcess *out, int capacity);
void *msutop_renderer_create(void);
void msutop_renderer_destroy(void *renderer);
void msutop_renderer_options(void *renderer, int vivid, int antialias, int demo);
void msutop_renderer_observe(void *renderer, const MonitorSample *sample, double seconds,
    uint64_t disk_used, uint64_t disk_total, const MonitorProcess *processes, int count);
int msutop_renderer_render(void *renderer, double seconds, uint8_t *rgba, int capacity);
void msutop_renderer_volumes(void *renderer, const MonitorVolume *volumes, int count);
int msutop_renderer_render_panels(void *renderer, double seconds, uint8_t *left, uint8_t *right, int capacity);
