#include "Metrics.h"
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <mach/mach.h>
#include <mach/processor_info.h>
#include <mach/mach_time.h>
#include <SystemConfiguration/SystemConfiguration.h>
#include <sys/sysctl.h>
#include <sys/resource.h>
#include <sys/time.h>
#include <sys/socket.h>
#include <net/if.h>
#include <net/if_dl.h>
#include <net/route.h>
#include <ifaddrs.h>
#include <libproc.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

static double seconds(void) {
    struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}
static double number(CFDictionaryRef dict, CFStringRef key, double fallback) {
    if (!dict) return fallback;
    CFTypeRef value = CFDictionaryGetValue(dict, key); double out;
    if (!value || CFGetTypeID(value) != CFNumberGetTypeID()) return fallback;
    return CFNumberGetValue((CFNumberRef)value, kCFNumberDoubleType, &out) ? out : fallback;
}
static void cpu(MonitorSample *s) {
    static uint32_t previous[MONITOR_MAX_CORES][CPU_STATE_MAX];
    static int previous_count;
    natural_t count = 0; processor_info_array_t info = NULL; mach_msg_type_number_t length = 0;
    if (host_processor_info(mach_host_self(), PROCESSOR_CPU_LOAD_INFO, &count, &info, &length) != KERN_SUCCESS) return;
    int n = count < MONITOR_MAX_CORES ? (int)count : MONITOR_MAX_CORES;
    double users = 0, systems = 0, totals = 0;
    for (int c = 0; c < n; c++) {
        double d[CPU_STATE_MAX], total = 0;
        for (int k = 0; k < CPU_STATE_MAX; k++) {
            uint32_t value = (uint32_t)info[c * CPU_STATE_MAX + k];
            d[k] = (uint32_t)(value - previous[c][k]);
            previous[c][k] = value; total += d[k];
        }
        if (previous_count == n && total > 0) {
            s->cores[c] = (total - d[CPU_STATE_IDLE]) * 100 / total;
            users += d[CPU_STATE_USER] + d[CPU_STATE_NICE];
            systems += d[CPU_STATE_SYSTEM]; totals += total;
        }
    }
    if (previous_count == n && totals > 0) {
        s->cpu_user = users * 100 / totals; s->cpu_system = systems * 100 / totals;
        s->cpu_available = 1;
    }
    s->core_count = n; previous_count = n;
    vm_deallocate(mach_task_self(), (vm_address_t)info, length * sizeof(integer_t));
}
static void memory(MonitorSample *s) {
    static uint64_t total;
    size_t size = sizeof(total);
    if (!total) sysctlbyname("hw.memsize", &total, &size, NULL, 0);
    s->memory_total = total;
    vm_statistics64_data_t vm; mach_msg_type_number_t count = HOST_VM_INFO64_COUNT;
    if (host_statistics64(mach_host_self(), HOST_VM_INFO64, (host_info64_t)&vm, &count) == KERN_SUCCESS) {
        vm_size_t page; host_page_size(mach_host_self(), &page);
        uint64_t app = vm.internal_page_count > vm.purgeable_count ? vm.internal_page_count - vm.purgeable_count : 0;
        s->memory_wired = (uint64_t)vm.wire_count * page;
        s->memory_compressed = (uint64_t)vm.compressor_page_count * page;
        s->memory_used = app * page + s->memory_wired + s->memory_compressed;
        if (s->memory_used > s->memory_total) s->memory_used = s->memory_total;
        s->memory_cached = ((uint64_t)vm.external_page_count + vm.purgeable_count) * page;
        s->memory_available = s->memory_total > 0;
    }
    static struct xsw_usage swap; static int swap_available; static double swap_time;
    double now = seconds();
    if (!swap_time || now - swap_time >= 0.25) {
        size = sizeof(swap); swap_available = sysctlbyname("vm.swapusage", &swap, &size, NULL, 0) == 0; swap_time = now;
    }
    if (swap_available) {
        s->swap_used = swap.xsu_used; s->swap_total = swap.xsu_total; s->swap_available = 1;
    }
}
static void gpu(MonitorSample *s) {
    io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOAccelerator"), &iterator) != KERN_SUCCESS) return;
    io_object_t service;
    while ((service = IOIteratorNext(iterator))) {
        CFTypeRef stats = IORegistryEntryCreateCFProperty(service, CFSTR("PerformanceStatistics"), kCFAllocatorDefault, 0);
        if (stats && CFGetTypeID(stats) == CFDictionaryGetTypeID()) {
            double usage = number((CFDictionaryRef)stats, CFSTR("Device Utilization %"), -1);
            if (usage >= 0 && usage <= 100) { s->gpu_usage = usage; s->gpu_available = 1; }
        }
        if (stats) CFRelease(stats);
        IOObjectRelease(service);
    }
    IOObjectRelease(iterator);
}
static void network(MonitorSample *s, double now) {
    typedef struct { char name[IFNAMSIZ]; uint64_t rx, tx; } Counter;
    static Counter previous[64]; static int previous_count; static double previous_time;
    Counter current[64]; int n = 0; double rx = 0, tx = 0;
    char primary[IFNAMSIZ] = "";
    static SCDynamicStoreRef store;
    if (!store) store = SCDynamicStoreCreate(NULL, CFSTR("msutop Mac"), NULL, NULL);
    const CFStringRef keys[] = { CFSTR("State:/Network/Global/IPv4"), CFSTR("State:/Network/Global/IPv6") };
    for (int k = 0; store && k < 2 && !primary[0]; k++) {
        CFPropertyListRef value = SCDynamicStoreCopyValue(store, keys[k]);
        if (value && CFGetTypeID(value) == CFDictionaryGetTypeID()) {
            CFTypeRef name = CFDictionaryGetValue((CFDictionaryRef)value, CFSTR("PrimaryInterface"));
            if (name && CFGetTypeID(name) == CFStringGetTypeID()) CFStringGetCString((CFStringRef)name, primary, sizeof(primary), kCFStringEncodingUTF8);
        }
        if (value) CFRelease(value);
    }
    int mib[] = { CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0 }; size_t length = 0;
    if (sysctl(mib, 6, NULL, &length, NULL, 0) != 0) return;
    char *buffer = malloc(length);
    if (!buffer) return;
    if (sysctl(mib, 6, buffer, &length, NULL, 0) != 0) { free(buffer); return; }
    for (char *p = buffer; p + sizeof(struct if_msghdr) <= buffer + length;) {
        struct if_msghdr *header = (struct if_msghdr *)p;
        if (!header->ifm_msglen || p + header->ifm_msglen > buffer + length) break;
        p += header->ifm_msglen;
        if (header->ifm_type != RTM_IFINFO2 || header->ifm_msglen < sizeof(struct if_msghdr2)) continue;
        struct if_msghdr2 *data = (struct if_msghdr2 *)header;
        char name[IFNAMSIZ] = "";
        // RTM_IFINFO2 already carries its interface name. if_indextoname() calls
        // getifaddrs() internally and would scan every interface again for every entry.
        struct sockaddr_dl *link = (struct sockaddr_dl *)(data + 1);
        if ((data->ifm_addrs & RTA_IFP) && (char *)link + sizeof(struct sockaddr_dl) <= (char *)header + header->ifm_msglen &&
            link->sdl_family == AF_LINK && link->sdl_nlen > 0 && link->sdl_nlen < IFNAMSIZ) {
            memcpy(name, link->sdl_data, link->sdl_nlen);
        } else {
            static char names[256][IFNAMSIZ];
            if (data->ifm_index >= 256) continue;
            if (!names[data->ifm_index][0] && !if_indextoname(data->ifm_index, names[data->ifm_index])) continue;
            snprintf(name, sizeof(name), "%s", names[data->ifm_index]);
        }
        // Prefer the physical default-route interface. Ignore VPN and bridge duplicates.
        if (strncmp(name, "en", 2) || !(data->ifm_flags & IFF_UP) || !(data->ifm_flags & IFF_RUNNING) || n == 64) continue;
        if (!strncmp(primary, "en", 2) && strcmp(primary, name)) continue;
        uint32_t mbps = (uint32_t)(data->ifm_data.ifi_baudrate / 1000000);
        if (mbps > s->link_mbps) s->link_mbps = mbps;
        snprintf(current[n].name, IFNAMSIZ, "%s", name);
        current[n].rx = data->ifm_data.ifi_ibytes; current[n].tx = data->ifm_data.ifi_obytes;
        for (int j = 0; j < previous_count; j++) {
            if (!strcmp(previous[j].name, current[n].name)) {
                if (current[n].rx >= previous[j].rx) rx += current[n].rx - previous[j].rx;
                if (current[n].tx >= previous[j].tx) tx += current[n].tx - previous[j].tx;
                break;
            }
        }
        size_t used = strlen(s->interfaces);
        snprintf(s->interfaces + used, sizeof(s->interfaces) - used, "%s%s", used ? "+" : "", name);
        n++;
    }
    free(buffer);
    double dt = now - previous_time;
    if (previous_time > 0 && dt > 0 && n > 0) { s->download = rx / dt; s->upload = tx / dt; s->network_available = 1; }
    memcpy(previous, current, n * sizeof(Counter)); previous_count = n; previous_time = now;
}
static void disk_io(MonitorSample *s, double now) {
    typedef struct { uint64_t id; double read, write; } DiskCounter;
    static DiskCounter previous[128]; static int previous_count; static double old_time;
    DiskCounter current[128]; int count = 0;
    double read = 0, write = 0; io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOBlockStorageDriver"), &iterator) != KERN_SUCCESS) return;
    io_object_t service;
    while ((service = IOIteratorNext(iterator))) {
        CFTypeRef stats = IORegistryEntryCreateCFProperty(service, CFSTR("Statistics"), kCFAllocatorDefault, 0);
        uint64_t id;
        if (stats && CFGetTypeID(stats) == CFDictionaryGetTypeID() && count < 128 && IORegistryEntryGetRegistryEntryID(service, &id) == KERN_SUCCESS) {
            current[count] = (DiskCounter){id, number((CFDictionaryRef)stats, CFSTR("Bytes (Read)"), 0), number((CFDictionaryRef)stats, CFSTR("Bytes (Write)"), 0)};
            for (int j = 0; j < previous_count; j++) {
                if (previous[j].id == id) {
                    if (current[count].read >= previous[j].read) read += current[count].read - previous[j].read;
                    if (current[count].write >= previous[j].write) write += current[count].write - previous[j].write;
                    break;
                }
            }
            count++;
        }
        if (stats) CFRelease(stats); IOObjectRelease(service);
    }
    IOObjectRelease(iterator);
    if (count && old_time && now > old_time) {
        s->disk_read = read / (now - old_time); s->disk_write = write / (now - old_time); s->disk_io_available = 1;
    }
    memcpy(previous, current, count * sizeof(DiskCounter)); previous_count = count; old_time = now;
}
void monitor_sample(MonitorSample *s) {
    memset(s, 0, sizeof(*s)); double now = seconds();
    cpu(s); memory(s); network(s, now);
    // Sample CPU/network at 30 Hz; IOKit sensors need only 4 Hz.
    static MonitorSample sensors; static double last_sensors;
    if (!last_sensors || now - last_sensors >= 0.25) {
        memset(&sensors, 0, sizeof(sensors)); gpu(&sensors); disk_io(&sensors, now); last_sensors = now;
    }
    s->gpu_usage = sensors.gpu_usage; s->gpu_available = sensors.gpu_available;
    s->disk_read = sensors.disk_read; s->disk_write = sensors.disk_write; s->disk_io_available = sensors.disk_io_available;
    static struct timeval boot; size_t size = sizeof(boot); struct timeval wall; gettimeofday(&wall, NULL);
    if (!boot.tv_sec) sysctlbyname("kern.boottime", &boot, &size, NULL, 0);
    if (boot.tv_sec) s->uptime = wall.tv_sec - boot.tv_sec;
}

typedef struct { int pid; uint64_t ticks, started; } ProcessCounter;
static int compare_pid(const void *a, const void *b) { int x = ((ProcessCounter *)a)->pid, y = ((ProcessCounter *)b)->pid; return (x > y) - (x < y); }
static int compare_cpu(const void *a, const void *b) { double x = ((MonitorProcess *)a)->cpu, y = ((MonitorProcess *)b)->cpu; return (y > x) - (y < x); }
int monitor_processes(MonitorProcess *out, int capacity) {
    static ProcessCounter *previous; static int previous_count; static double previous_time;
    int bytes = proc_listpids(PROC_ALL_PIDS, 0, NULL, 0);
    if (bytes <= 0 || capacity <= 0) return 0;
    bytes += 4096 * sizeof(pid_t); pid_t *pids = malloc(bytes);
    if (!pids) return 0;
    int count = proc_listpids(PROC_ALL_PIDS, 0, pids, bytes) / sizeof(pid_t);
    if (count <= 0) { free(pids); return 0; }
    ProcessCounter *current = calloc(count, sizeof(*current));
    MonitorProcess *ranked = calloc(count, sizeof(*ranked));
    if (!current || !ranked) { free(pids); free(current); free(ranked); return 0; }
    double now = seconds(), dt = now - previous_time; int n = 0, ranked_count = 0;
    mach_timebase_info_data_t timebase; mach_timebase_info(&timebase);
    for (int i = 0; i < count; i++) {
        if (pids[i] <= 0) continue;
        struct rusage_info_v2 usage;
        if (proc_pid_rusage(pids[i], RUSAGE_INFO_V2, (rusage_info_t *)&usage) != 0) continue;
        current[n] = (ProcessCounter){pids[i], usage.ri_user_time + usage.ri_system_time, usage.ri_proc_start_abstime};
        ProcessCounter *old = previous_count ? bsearch(&current[n], previous, previous_count, sizeof(*previous), compare_pid) : NULL;
        if (old && old->started == current[n].started && dt > 0 && current[n].ticks >= old->ticks) {
            MonitorProcess *r = &ranked[ranked_count++]; r->pid = pids[i];
            // proc_pid_rusage reports Mach absolute-time ticks, not nanoseconds on Apple Silicon.
            r->cpu = (current[n].ticks - old->ticks) * ((double)timebase.numer / timebase.denom) / 1e9 / dt * 100;
            r->memory = usage.ri_resident_size;
            r->identity = ((uint64_t)pids[i] << 32) ^ usage.ri_proc_start_abstime;
            if (!proc_name(pids[i], r->name, sizeof(r->name))) snprintf(r->name, sizeof(r->name), "pid %d", pids[i]);
        }
        n++;
    }
    qsort(current, n, sizeof(*current), compare_pid); qsort(ranked, ranked_count, sizeof(*ranked), compare_cpu);
    int result = ranked_count < capacity ? ranked_count : capacity; memcpy(out, ranked, result * sizeof(*out));
    free(previous); previous = current; previous_count = n; previous_time = now;
    free(pids); free(ranked); return result;
}
