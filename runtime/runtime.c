/*
 * _XOPEN_SOURCE 700 is required to get ucontext_t on macOS/Linux.
 * It hides some BSD/GNU extensions, so we forward-declare them where needed.
 */
#if !defined(_WIN32) && !defined(_WIN64)
#  define _XOPEN_SOURCE 700
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <time.h>
#include <setjmp.h>
#include <stdint.h>

/* Platform-specific headers */
#if defined(_WIN32) || defined(_WIN64)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
#elif defined(__APPLE__) || defined(__linux__)
#  include <ucontext.h>
/* arc4random_buf is not declared under _XOPEN_SOURCE on macOS — forward-declare it. */
#  if defined(__APPLE__)
     extern void arc4random_buf(void* buf, size_t nbytes);
#  endif
#  if defined(__linux__)
#    include <sys/syscall.h>
#    include <unistd.h>
#  endif
#endif

/* (sys/syscall.h and unistd.h are included in the platform block above for Linux) */

// ============================================================
// String type
// ============================================================
typedef struct {
    char* data;
    long long len;
} MgString;

// ============================================================
// Print functions
// ============================================================
void tscc_print_number(double n) {
    if (n == (long long)n && !isinf(n) && fabs(n) < 1e15) {
        printf("%lld", (long long)n);
    } else {
        printf("%.15g", n);
    }
}

void tscc_print_string(char* data, long long len) {
    fwrite(data, 1, (size_t)len, stdout);
}

void tscc_print_boolean(int b) {
    printf(b ? "true" : "false");
}

void tscc_print_null(void) {
    printf("null");
}

void tscc_print_undefined(void) {
    printf("undefined");
}

void tscc_print_newline(void) {
    printf("\n");
}

// console.error / console.warn write to stderr
void tscc_eprint_number(double n) {
    if (n == (long long)n && !isinf(n) && fabs(n) < 1e15) {
        fprintf(stderr, "%lld", (long long)n);
    } else {
        fprintf(stderr, "%.15g", n);
    }
}

void tscc_eprint_string(char* data, long long len) {
    fwrite(data, 1, (size_t)len, stderr);
}

void tscc_eprint_boolean(int b) {
    fprintf(stderr, b ? "true" : "false");
}

void tscc_eprint_newline(void) {
    fprintf(stderr, "\n");
}

// ============================================================
// String operations
// ============================================================
MgString tscc_string_concat(char* a_data, long long a_len, char* b_data, long long b_len) {
    long long new_len = a_len + b_len;
    char* new_data = (char*)malloc((size_t)(new_len + 1));
    if (!new_data) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    memcpy(new_data, a_data, (size_t)a_len);
    memcpy(new_data + a_len, b_data, (size_t)b_len);
    new_data[new_len] = '\0';
    MgString result = { new_data, new_len };
    return result;
}

MgString tscc_number_to_string(double n) {
    char buf[64];
    int len;
    if (n == (long long)n && !isinf(n) && fabs(n) < 1e15) {
        len = snprintf(buf, sizeof(buf), "%lld", (long long)n);
    } else {
        len = snprintf(buf, sizeof(buf), "%.15g", n);
    }
    char* data = (char*)malloc((size_t)(len + 1));
    if (!data) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    memcpy(data, buf, (size_t)(len + 1));
    MgString result = { data, len };
    return result;
}

MgString tscc_boolean_to_string(int b) {
    if (b) {
        MgString r = { "true", 4 }; return r;
    } else {
        MgString r = { "false", 5 }; return r;
    }
}

// ============================================================
// String methods
// ============================================================
MgString tscc_string_toUpperCase(char* data, long long len) {
    char* out = (char*)malloc((size_t)(len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    for (long long i = 0; i < len; i++) out[i] = (char)toupper((unsigned char)data[i]);
    out[len] = '\0';
    MgString r = { out, len }; return r;
}

MgString tscc_string_toLowerCase(char* data, long long len) {
    char* out = (char*)malloc((size_t)(len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    for (long long i = 0; i < len; i++) out[i] = (char)tolower((unsigned char)data[i]);
    out[len] = '\0';
    MgString r = { out, len }; return r;
}

MgString tscc_string_charAt(char* data, long long len, double index) {
    long long idx = (long long)index;
    if (idx < 0 || idx >= len) {
        MgString r = { "", 0 }; return r;
    }
    char* out = (char*)malloc(2);
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    out[0] = data[idx]; out[1] = '\0';
    MgString r = { out, 1 }; return r;
}

double tscc_string_indexOf(char* haystack, long long hay_len, char* needle, long long needle_len) {
    if (needle_len == 0) return 0;
    if (needle_len > hay_len) return -1;
    for (long long i = 0; i <= hay_len - needle_len; i++) {
        if (memcmp(&haystack[i], needle, (size_t)needle_len) == 0) return (double)i;
    }
    return -1;
}

int tscc_string_includes(char* haystack, long long hay_len, char* needle, long long needle_len) {
    return tscc_string_indexOf(haystack, hay_len, needle, needle_len) >= 0 ? 1 : 0;
}

MgString tscc_string_substring(char* data, long long len, double start_d, double end_d) {
    long long start = (long long)start_d;
    long long end = (long long)end_d;
    if (start < 0) start = 0;
    if (end > len) end = len;
    if (start > end) { long long t = start; start = end; end = t; }
    long long sub_len = end - start;
    char* out = (char*)malloc((size_t)(sub_len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    memcpy(out, &data[start], (size_t)sub_len);
    out[sub_len] = '\0';
    MgString r = { out, sub_len }; return r;
}

MgString tscc_string_slice(char* data, long long len, double start_d, double end_d) {
    long long start = (long long)start_d;
    long long end = (long long)end_d;
    if (start < 0) { start = len + start; if (start < 0) start = 0; }
    if (end < 0) { end = len + end; if (end < 0) end = 0; }
    if (end > len) end = len;
    if (start >= end) { MgString r = { "", 0 }; return r; }
    long long sub_len = end - start;
    char* out = (char*)malloc((size_t)(sub_len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    memcpy(out, &data[start], (size_t)sub_len);
    out[sub_len] = '\0';
    MgString r = { out, sub_len }; return r;
}

MgString tscc_string_trim(char* data, long long len) {
    long long start = 0, end = len;
    while (start < len && isspace((unsigned char)data[start])) start++;
    while (end > start && isspace((unsigned char)data[end - 1])) end--;
    long long new_len = end - start;
    char* out = (char*)malloc((size_t)(new_len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    memcpy(out, &data[start], (size_t)new_len);
    out[new_len] = '\0';
    MgString r = { out, new_len }; return r;
}

int tscc_string_startsWith(char* haystack, long long hay_len, char* needle, long long needle_len) {
    if (needle_len > hay_len) return 0;
    if (needle_len == 0) return 1;
    return memcmp(haystack, needle, (size_t)needle_len) == 0 ? 1 : 0;
}

int tscc_string_endsWith(char* haystack, long long hay_len, char* needle, long long needle_len) {
    if (needle_len > hay_len) return 0;
    if (needle_len == 0) return 1;
    return memcmp(&haystack[hay_len - needle_len], needle, (size_t)needle_len) == 0 ? 1 : 0;
}

MgString tscc_string_repeat(char* data, long long len, double count_d) {
    long long count = (long long)count_d;
    if (count <= 0 || len == 0) {
        MgString r = { "", 0 }; return r;
    }
    long long new_len = len * count;
    char* out = (char*)malloc((size_t)(new_len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    for (long long i = 0; i < count; i++) {
        memcpy(&out[i * len], data, (size_t)len);
    }
    out[new_len] = '\0';
    MgString r = { out, new_len }; return r;
}

MgString tscc_string_replace(char* data, long long len,
                             char* search, long long search_len,
                             char* replace, long long replace_len) {
    if (search_len == 0 || search_len > len) {
        char* out = (char*)malloc((size_t)(len + 1));
        if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
        memcpy(out, data, (size_t)len);
        out[len] = '\0';
        MgString r = { out, len }; return r;
    }
    // Find first occurrence
    long long pos = -1;
    for (long long i = 0; i <= len - search_len; i++) {
        if (memcmp(&data[i], search, (size_t)search_len) == 0) { pos = i; break; }
    }
    if (pos < 0) {
        char* out = (char*)malloc((size_t)(len + 1));
        if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
        memcpy(out, data, (size_t)len);
        out[len] = '\0';
        MgString r = { out, len }; return r;
    }
    long long new_len = len - search_len + replace_len;
    char* out = (char*)malloc((size_t)(new_len + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    memcpy(out, data, (size_t)pos);
    memcpy(&out[pos], replace, (size_t)replace_len);
    memcpy(&out[pos + replace_len], &data[pos + search_len],
           (size_t)(len - pos - search_len));
    out[new_len] = '\0';
    MgString r = { out, new_len }; return r;
}

MgString tscc_string_padStart(char* data, long long len, double target_d,
                              char* pad_data, long long pad_len) {
    long long target = (long long)target_d;
    if (target <= len || pad_len == 0) {
        char* out = (char*)malloc((size_t)(len + 1));
        if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
        memcpy(out, data, (size_t)len);
        out[len] = '\0';
        MgString r = { out, len }; return r;
    }
    long long pad_total = target - len;
    char* out = (char*)malloc((size_t)(target + 1));
    if (!out) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    long long pos = 0;
    while (pos < pad_total) {
        long long copy_len = pad_len;
        if (pos + copy_len > pad_total) copy_len = pad_total - pos;
        memcpy(&out[pos], pad_data, (size_t)copy_len);
        pos += copy_len;
    }
    memcpy(&out[pad_total], data, (size_t)len);
    out[target] = '\0';
    MgString r = { out, target }; return r;
}

// ============================================================
// Math functions
// ============================================================
static int _tscc_rng_seeded = 0;

double tscc_math_floor(double x) { return floor(x); }
double tscc_math_ceil(double x) { return ceil(x); }
double tscc_math_round(double x) { return round(x); }
double tscc_math_abs(double x) { return fabs(x); }
double tscc_math_sqrt(double x) { return sqrt(x); }
double tscc_math_pow(double x, double y) { return pow(x, y); }
double tscc_math_min(double a, double b) { return a < b ? a : b; }
double tscc_math_max(double a, double b) { return a > b ? a : b; }
double tscc_math_sin(double x) { return sin(x); }
double tscc_math_cos(double x) { return cos(x); }
double tscc_math_tan(double x) { return tan(x); }
double tscc_math_log(double x) { return log(x); }
double tscc_math_exp(double x) { return exp(x); }

double tscc_math_random(void) {
    if (!_tscc_rng_seeded) {
        srand((unsigned int)time(NULL));
        _tscc_rng_seeded = 1;
    }
    return (double)rand() / (double)RAND_MAX;
}

// ============================================================
// Array functions
// ============================================================
typedef struct {
    double* data;
    long long length;
    long long capacity;
} MgArray;

// Takes a pointer to the array struct and modifies it in place
void tscc_array_push(MgArray* arr, double value) {
    if (arr->length >= arr->capacity) {
        arr->capacity = arr->capacity < 4 ? 4 : arr->capacity * 2;
        arr->data = (double*)realloc(arr->data, sizeof(double) * (size_t)arr->capacity);
        if (!arr->data) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    }
    arr->data[arr->length++] = value;
}

// Object-array: same layout as MgArray but data is void** (pointers to heap objects).
// This is a parallel to tscc_array_push for arrays of object references.
typedef struct {
    void** data;
    long long length;
    long long capacity;
} MgObjArray;

void tscc_obj_array_push(MgObjArray* arr, void* elem) {
    if (arr->length >= arr->capacity) {
        arr->capacity = arr->capacity < 4 ? 4 : arr->capacity * 2;
        arr->data = (void**)realloc(arr->data, sizeof(void*) * (size_t)arr->capacity);
        if (!arr->data) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    }
    arr->data[arr->length++] = elem;
}

void tscc_print_array(double* data, long long length) {
    printf("[ ");
    for (long long i = 0; i < length; i++) {
        if (i > 0) printf(", ");
        tscc_print_number(data[i]);
    }
    printf(" ]");
}

void tscc_eprint_array(double* data, long long length) {
    fprintf(stderr, "[ ");
    for (long long i = 0; i < length; i++) {
        if (i > 0) fprintf(stderr, ", ");
        tscc_eprint_number(data[i]);
    }
    fprintf(stderr, " ]");
}

// ============================================================
// String array support
// ============================================================

// String array: array of {char*, long long} structs
// Returns via out-parameters: *out_data = heap-allocated array of MgString, *out_len = count
void tscc_string_split(char* data, long long len, char* sep_data, long long sep_len,
                       MgString** out_data, long long* out_len) {
    if (len == 0) {
        // Empty string splits to [""]
        MgString* arr = (MgString*)malloc(sizeof(MgString));
        arr[0].data = (char*)malloc(1);
        arr[0].data[0] = '\0';
        arr[0].len = 0;
        *out_data = arr;
        *out_len = 1;
        return;
    }

    // Count occurrences first
    long long count = 1;
    for (long long i = 0; i <= len - sep_len; i++) {
        if (memcmp(data + i, sep_data, (size_t)sep_len) == 0) {
            count++;
            i += sep_len - 1;
        }
    }

    MgString* arr = (MgString*)malloc((size_t)count * sizeof(MgString));
    long long idx = 0;
    long long start = 0;
    for (long long i = 0; i <= len - sep_len; i++) {
        if (memcmp(data + i, sep_data, (size_t)sep_len) == 0) {
            long long part_len = i - start;
            arr[idx].data = (char*)malloc((size_t)(part_len + 1));
            memcpy(arr[idx].data, data + start, (size_t)part_len);
            arr[idx].data[part_len] = '\0';
            arr[idx].len = part_len;
            idx++;
            i += sep_len - 1;
            start = i + 1;
        }
    }
    // Last part
    long long part_len = len - start;
    arr[idx].data = (char*)malloc((size_t)(part_len + 1));
    memcpy(arr[idx].data, data + start, (size_t)part_len);
    arr[idx].data[part_len] = '\0';
    arr[idx].len = part_len;

    *out_data = arr;
    *out_len = count;
}

void tscc_print_string_array(MgString* data, long long length) {
    printf("[ ");
    for (long long i = 0; i < length; i++) {
        if (i > 0) printf(", ");
        printf("'");
        fwrite(data[i].data, 1, (size_t)data[i].len, stdout);
        printf("'");
    }
    printf(" ]");
}

// ============================================================
// Number methods
// ============================================================

void tscc_number_toFixed(double value, double digits, char** out_data, long long* out_len) {
    int d = (int)digits;
    if (d < 0) d = 0;
    if (d > 100) d = 100;
    char buf[256];
    int n = snprintf(buf, sizeof(buf), "%.*f", d, value);
    *out_data = (char*)malloc((size_t)(n + 1));
    memcpy(*out_data, buf, (size_t)(n + 1));
    *out_len = n;
}

double tscc_number_isFinite(double value) {
    return isfinite(value) ? 1.0 : 0.0;
}

double tscc_number_isInteger(double value) {
    return (isfinite(value) && floor(value) == value) ? 1.0 : 0.0;
}

double tscc_number_isNaN(double value) {
    return isnan(value) ? 1.0 : 0.0;
}

// ============================================================
// Map functions (Map<K, V> — string keys, arbitrary value blobs)
// ============================================================
typedef struct {
    char**      keys;
    long long*  key_lens;
    void**      values;     // malloc'd copies of each value
    long long   count;
    long long   capacity;
} MgMap;

MgMap* tscc_map_alloc() {
    MgMap* m = (MgMap*)malloc(sizeof(MgMap));
    m->keys     = NULL;
    m->key_lens = NULL;
    m->values   = NULL;
    m->count    = 0;
    m->capacity = 0;
    return m;
}

static long long tscc_map_find(MgMap* m, char* key, long long klen) {
    for (long long i = 0; i < m->count; i++) {
        if (m->key_lens[i] == klen && memcmp(m->keys[i], key, (size_t)klen) == 0)
            return i;
    }
    return -1;
}

void tscc_map_set(MgMap* m, char* key, long long klen, void* val, long long vsize) {
    long long idx = tscc_map_find(m, key, klen);
    if (idx >= 0) {
        memcpy(m->values[idx], val, (size_t)vsize);
        return;
    }
    if (m->count >= m->capacity) {
        m->capacity = m->capacity < 4 ? 4 : m->capacity * 2;
        m->keys     = (char**)realloc(m->keys,     (size_t)m->capacity * sizeof(char*));
        m->key_lens = (long long*)realloc(m->key_lens, (size_t)m->capacity * sizeof(long long));
        m->values   = (void**)realloc(m->values,   (size_t)m->capacity * sizeof(void*));
    }
    m->keys[m->count] = (char*)malloc((size_t)(klen + 1));
    memcpy(m->keys[m->count], key, (size_t)klen);
    m->keys[m->count][klen] = '\0';
    m->key_lens[m->count] = klen;
    m->values[m->count]   = malloc((size_t)vsize);
    memcpy(m->values[m->count], val, (size_t)vsize);
    m->count++;
}

void* tscc_map_get(MgMap* m, char* key, long long klen) {
    long long idx = tscc_map_find(m, key, klen);
    return idx >= 0 ? m->values[idx] : NULL;
}

int tscc_map_has(MgMap* m, char* key, long long klen) {
    return tscc_map_find(m, key, klen) >= 0 ? 1 : 0;
}

int tscc_map_delete(MgMap* m, char* key, long long klen) {
    long long idx = tscc_map_find(m, key, klen);
    if (idx < 0) return 0;
    free(m->keys[idx]);
    free(m->values[idx]);
    for (long long i = idx; i < m->count - 1; i++) {
        m->keys[i]     = m->keys[i + 1];
        m->key_lens[i] = m->key_lens[i + 1];
        m->values[i]   = m->values[i + 1];
    }
    m->count--;
    return 1;
}

long long tscc_map_size(MgMap* m) { return m->count; }

// Returns a newly malloc'd void** of value pointers; writes count to *out_count
void** tscc_map_values_alloc(MgMap* m, long long* out_count) {
    *out_count = m->count;
    if (m->count == 0) return NULL;
    void** ptrs = (void**)malloc((size_t)m->count * sizeof(void*));
    for (long long i = 0; i < m->count; i++) ptrs[i] = m->values[i];
    return ptrs;
}

// ============================================================
// Date
// ============================================================

/* Returns milliseconds since Unix epoch (UTC). */
long long tscc_date_now(void) {
    struct timespec ts;
#if defined(_WIN32) || defined(_WIN64)
    /* Windows: use GetSystemTimeAsFileTime for ms precision */
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    long long t = ((long long)ft.dwHighDateTime << 32) | (long long)ft.dwLowDateTime;
    /* Convert Windows epoch (1601-01-01) to Unix epoch (1970-01-01): 116444736000000000 100-ns ticks */
    t -= 116444736000000000LL;
    return t / 10000LL;  /* 100-ns to ms */
#elif defined(_POSIX_C_SOURCE) || defined(__APPLE__) || defined(__linux__)
    clock_gettime(CLOCK_REALTIME, &ts);
    return (long long)ts.tv_sec * 1000LL + (long long)ts.tv_nsec / 1000000LL;
#else
    return (long long)time(NULL) * 1000LL;
#endif
}

/* getTime() — same as the stored ms value. */
long long tscc_date_get_time(long long ms) { return ms; }

/* Local-time getters — decompose ms via localtime. */
static struct tm tscc_ms_to_local(long long ms) {
    time_t sec = (time_t)(ms / 1000);
    struct tm t;
#if defined(_WIN32) || defined(_WIN64)
    localtime_s(&t, &sec);
#else
    localtime_r(&sec, &t);
#endif
    return t;
}

/* UTC getters — decompose ms via gmtime. */
static struct tm tscc_ms_to_utc(long long ms) {
    time_t sec = (time_t)(ms / 1000);
    struct tm t;
#if defined(_WIN32) || defined(_WIN64)
    gmtime_s(&t, &sec);
#else
    gmtime_r(&sec, &t);
#endif
    return t;
}

long long tscc_date_get_full_year(long long ms)     { return (long long)(tscc_ms_to_local(ms).tm_year + 1900); }
long long tscc_date_get_month(long long ms)         { return (long long)tscc_ms_to_local(ms).tm_mon; }   /* 0-indexed */
long long tscc_date_get_date(long long ms)          { return (long long)tscc_ms_to_local(ms).tm_mday; }  /* 1-indexed */
long long tscc_date_get_hours(long long ms)         { return (long long)tscc_ms_to_local(ms).tm_hour; }
long long tscc_date_get_minutes(long long ms)       { return (long long)tscc_ms_to_local(ms).tm_min; }
long long tscc_date_get_seconds(long long ms)       { return (long long)tscc_ms_to_local(ms).tm_sec; }
long long tscc_date_get_milliseconds(long long ms)  { return ms % 1000LL; }

long long tscc_date_get_utc_full_year(long long ms)    { return (long long)(tscc_ms_to_utc(ms).tm_year + 1900); }
long long tscc_date_get_utc_month(long long ms)        { return (long long)tscc_ms_to_utc(ms).tm_mon; }
long long tscc_date_get_utc_date(long long ms)         { return (long long)tscc_ms_to_utc(ms).tm_mday; }
long long tscc_date_get_utc_hours(long long ms)        { return (long long)tscc_ms_to_utc(ms).tm_hour; }
long long tscc_date_get_utc_minutes(long long ms)      { return (long long)tscc_ms_to_utc(ms).tm_min; }
long long tscc_date_get_utc_seconds(long long ms)      { return (long long)tscc_ms_to_utc(ms).tm_sec; }
long long tscc_date_get_utc_milliseconds(long long ms) { return ms % 1000LL; }

/* toISOString() — "YYYY-MM-DDTHH:MM:SS.mmmZ" (always UTC, 24 chars) */
MgString tscc_date_to_iso_string(long long ms) {
    struct tm t = tscc_ms_to_utc(ms);
    int millis = (int)(ms < 0 ? (1000 + ms % 1000) % 1000 : ms % 1000);
    char* buf = (char*)malloc(25);
    snprintf(buf, 25, "%04d-%02d-%02dT%02d:%02d:%02d.%03dZ",
             t.tm_year + 1900, t.tm_mon + 1, t.tm_mday,
             t.tm_hour, t.tm_min, t.tm_sec, millis);
    MgString s; s.data = buf; s.len = 24;
    return s;
}

// ============================================================
// Global functions
// ============================================================
double tscc_parseInt(char* data, long long len) {
    if (len == 0) return NAN;
    char buf[128];
    long long copy_len = len < 127 ? len : 127;
    memcpy(buf, data, (size_t)copy_len);
    buf[copy_len] = '\0';
    // Skip leading whitespace
    char* p = buf;
    while (*p && isspace((unsigned char)*p)) p++;
    if (*p == '\0') return NAN;
    char* endptr;
    long long val = strtoll(p, &endptr, 10);
    if (endptr == p) return NAN;
    return (double)val;
}

double tscc_parseFloat(char* data, long long len) {
    if (len == 0) return NAN;
    char buf[128];
    long long copy_len = len < 127 ? len : 127;
    memcpy(buf, data, (size_t)copy_len);
    buf[copy_len] = '\0';
    char* p = buf;
    while (*p && isspace((unsigned char)*p)) p++;
    if (*p == '\0') return NAN;
    char* endptr;
    double val = strtod(p, &endptr);
    if (endptr == p) return NAN;
    return val;
}

// ============================================================
// Crypto
// ============================================================

/* Fill buf with len cryptographically secure random bytes.
 *
 * Windows       – BCryptGenRandom via dynamic load (bcrypt.dll, guaranteed
 *                 present on Vista+); no extra link flags needed.
 * macOS / BSDs  – arc4random_buf (stdlib.h, already included, no seeding).
 * Linux 3.17+   – getrandom() syscall (SYS_getrandom).
 * Other Unix    – /dev/urandom fallback.
 */
static void tscc_csprng_fill(unsigned char* buf, size_t len) {
#if defined(_WIN32) || defined(_WIN64)
    typedef long (__stdcall *pBCryptGenRandom)(void*, unsigned char*, unsigned long, unsigned long);
    HMODULE h = LoadLibraryA("bcrypt.dll");
    if (h) {
        pBCryptGenRandom fn = (pBCryptGenRandom)GetProcAddress(h, "BCryptGenRandom");
        if (fn) fn(NULL, buf, (unsigned long)len, 2); /* 2 = BCRYPT_USE_SYSTEM_PREFERRED_RNG */
        FreeLibrary(h);
    }
#elif defined(__APPLE__) || defined(__FreeBSD__) || defined(__OpenBSD__) || defined(__NetBSD__)
    arc4random_buf(buf, len);
#elif defined(__linux__) && defined(SYS_getrandom)
    if (syscall(SYS_getrandom, buf, (long)len, 0L) == (long)len) return;
    /* fallthrough: getrandom unavailable, use /dev/urandom */
    { FILE* f = fopen("/dev/urandom", "rb"); if (f) { fread(buf, 1, len, f); fclose(f); } }
#else
    FILE* f = fopen("/dev/urandom", "rb");
    if (f) { fread(buf, 1, len, f); fclose(f); }
#endif
}

/* crypto.randomUUID() — RFC 4122 UUID v4 */
MgString tscc_crypto_random_uuid(void) {
    unsigned char b[16];
    tscc_csprng_fill(b, 16);
    b[6] = (b[6] & 0x0f) | 0x40;  /* version 4          */
    b[8] = (b[8] & 0x3f) | 0x80;  /* variant 10xx (RFC) */
    char* buf = (char*)malloc(37);
    snprintf(buf, 37,
        "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
        b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7],
        b[8],b[9],b[10],b[11],b[12],b[13],b[14],b[15]);
    MgString s; s.data = buf; s.len = 36;
    return s;
}

// ============================================================
// Exception handling (setjmp / longjmp based try/catch/throw)
// ============================================================

typedef struct TsccHandler {
    jmp_buf        jb;
    void*          thrown_value;
} TsccHandler;

#define TSCC_MAX_HANDLERS 1024
static TsccHandler tscc_handlers[TSCC_MAX_HANDLERS];
static int         tscc_handler_top = 0;

/* Push a new handler and return its jmp_buf pointer for use with setjmp(). */
jmp_buf* tscc_try_enter(void) {
    if (tscc_handler_top >= TSCC_MAX_HANDLERS) {
        fprintf(stderr, "tscc: exception handler stack overflow\n");
        exit(1);
    }
    return &tscc_handlers[tscc_handler_top++].jb;
}

/* Normal exit from a try body — pop the handler. */
void tscc_try_exit(void) {
    if (tscc_handler_top > 0) tscc_handler_top--;
}

/* Retrieve the thrown value after setjmp returns non-zero.
 * tscc_throw() has already decremented tscc_handler_top. */
void* tscc_catch_value(void) {
    return tscc_handlers[tscc_handler_top].thrown_value;
}

/* Throw an exception — longjmp to the nearest try handler. */
void tscc_throw(void* value) {
    if (tscc_handler_top == 0) {
        fprintf(stderr, "Uncaught exception\n");
        exit(1);
    }
    tscc_handler_top--;
    tscc_handlers[tscc_handler_top].thrown_value = value;
    longjmp(tscc_handlers[tscc_handler_top].jb, 1);
}

/* Box a double on the heap so it can be passed as void*. */
void* tscc_box_number(double v) {
    double* p = (double*)malloc(sizeof(double));
    if (!p) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    *p = v;
    return p;
}
double tscc_unbox_number(void* p) { return *(double*)p; }

/* Box/unbox boolean (stored as intptr_t). */
void* tscc_box_boolean(int v) { return (void*)(intptr_t)(v ? 1 : 0); }
int   tscc_unbox_boolean(void* p) { return (int)(intptr_t)p; }

/* Box a string by copying the MgString header to the heap. */
typedef struct { char* data; long long len; } MgStringBox;
void* tscc_box_string(char* data, long long len) {
    MgStringBox* p = (MgStringBox*)malloc(sizeof(MgStringBox));
    if (!p) { fprintf(stderr, "tscc: out of memory\n"); exit(1); }
    p->data = data; p->len = len;
    return p;
}
/* Returns a heap-allocated MgString (for use after unboxing) */
MgString tscc_unbox_string(void* p) {
    MgStringBox* b = (MgStringBox*)p;
    MgString s; s.data = b->data; s.len = b->len;
    return s;
}

// ============================================================
// Fibers (stackful coroutines)
// ============================================================

#define TSCC_FIBER_STACK_SIZE (512 * 1024)   /* 512 KiB per fiber */

typedef struct MgFiber MgFiber;

struct MgFiber {
#if defined(_WIN32) || defined(_WIN64)
    LPVOID           fiber_handle;   /* Windows fiber handle */
    LPVOID           caller_handle;  /* caller's fiber handle */
#else
    ucontext_t       uc;             /* POSIX fiber context */
    ucontext_t       caller_uc;      /* saved caller context */
    void*            stack;          /* mmap'd stack */
    size_t           stack_size;
#endif
    void           (*fn)(void*);     /* fiber entry function */
    void*            arg;            /* argument to fn */
    int              finished;       /* 1 once fn returns */
    struct MgFiber*  resumer;        /* fiber to resume when this one yields */
};

/* Current running fiber (NULL = main execution context). */
static MgFiber* tscc_current_fiber = NULL;

#if defined(_WIN32) || defined(_WIN64)

/* Windows fiber trampoline */
static VOID WINAPI tscc_fiber_trampoline(LPVOID lpParam) {
    MgFiber* f = (MgFiber*)lpParam;
    f->fn(f->arg);
    f->finished = 1;
    SwitchToFiber(f->caller_handle);
}

MgFiber* tscc_fiber_create(void (*fn)(void*), void* arg) {
    MgFiber* f = (MgFiber*)calloc(1, sizeof(MgFiber));
    f->fn  = fn;
    f->arg = arg;
    f->fiber_handle = CreateFiber(TSCC_FIBER_STACK_SIZE, tscc_fiber_trampoline, f);
    return f;
}

void tscc_fiber_resume(MgFiber* f) {
    MgFiber* prev = tscc_current_fiber;
    tscc_current_fiber = f;
    if (f->caller_handle == NULL) {
        /* First resume: convert current thread to a fiber if needed */
        if (prev == NULL) {
            /* main thread */
            LPVOID main_fiber = ConvertThreadToFiber(NULL);
            f->caller_handle = main_fiber;
        } else {
            f->caller_handle = prev->fiber_handle;
        }
    }
    SwitchToFiber(f->fiber_handle);
    tscc_current_fiber = prev;
}

void tscc_fiber_yield(void) {
    MgFiber* f = tscc_current_fiber;
    if (!f) return;
    SwitchToFiber(f->caller_handle);
}

void tscc_fiber_destroy(MgFiber* f) {
    if (f) { DeleteFiber(f->fiber_handle); free(f); }
}

#else /* POSIX (macOS + Linux) */

static void tscc_fiber_trampoline(uint32_t hi, uint32_t lo) {
    uintptr_t ptr = ((uintptr_t)hi << 32) | (uintptr_t)lo;
    MgFiber* f = (MgFiber*)ptr;
    f->fn(f->arg);
    f->finished = 1;
    /* Return to caller */
    swapcontext(&f->uc, &f->caller_uc);
}

MgFiber* tscc_fiber_create(void (*fn)(void*), void* arg) {
    MgFiber* f = (MgFiber*)calloc(1, sizeof(MgFiber));
    f->fn         = fn;
    f->arg        = arg;
    f->stack_size = TSCC_FIBER_STACK_SIZE;
    /* Use malloc for portability — no mmap / MAP_ANONYMOUS needed */
    f->stack      = malloc(f->stack_size);
    if (!f->stack) {
        fprintf(stderr, "tscc: fiber stack allocation failed\n"); exit(1);
    }
    getcontext(&f->uc);
    f->uc.uc_stack.ss_sp   = f->stack;
    f->uc.uc_stack.ss_size = f->stack_size;
    f->uc.uc_link          = NULL;  /* we manage return manually */
    /* Pass the fiber pointer as two 32-bit halves (portable for 64-bit) */
    uintptr_t ptr = (uintptr_t)f;
    uint32_t hi = (uint32_t)(ptr >> 32);
    uint32_t lo = (uint32_t)(ptr & 0xFFFFFFFF);
    makecontext(&f->uc, (void(*)(void))tscc_fiber_trampoline, 2, hi, lo);
    return f;
}

void tscc_fiber_resume(MgFiber* f) {
    MgFiber* prev = tscc_current_fiber;
    tscc_current_fiber = f;
    swapcontext(&f->caller_uc, &f->uc);
    tscc_current_fiber = prev;
}

void tscc_fiber_yield(void) {
    MgFiber* f = tscc_current_fiber;
    if (!f) return;
    swapcontext(&f->uc, &f->caller_uc);
}

void tscc_fiber_destroy(MgFiber* f) {
    if (!f) return;
    free(f->stack);
    free(f);
}

#endif /* POSIX */

MgFiber* tscc_fiber_current(void) { return tscc_current_fiber; }

// ============================================================
// Promise
// ============================================================

typedef void (*MgPromiseCallback)(void* value, void* ctx);

typedef struct MgThenNode {
    MgPromiseCallback cb;
    void*             ctx;
    struct MgPromise* next_promise;  /* chained promise */
    struct MgThenNode* next;
} MgThenNode;

typedef struct MgPromise {
    int         state;          /* 0=pending, 1=fulfilled, 2=rejected */
    void*       value;          /* resolved value or rejection reason */
    MgThenNode* then_head;      /* linked list of .then callbacks */
    MgThenNode* catch_head;     /* linked list of .catch callbacks */
    MgFiber*    waiting_fiber;  /* fiber blocked on this promise */
    int         fiber_rejected; /* 1 if the waiting fiber should receive a rejection */
} MgPromise;

static void tscc_microtask_enqueue(MgPromiseCallback cb, void* val, void* ctx,
                                   MgPromise* chained);

MgPromise* tscc_promise_new(void) {
    MgPromise* p = (MgPromise*)calloc(1, sizeof(MgPromise));
    return p;
}

static void tscc_fire_callbacks(MgPromise* p) {
    MgThenNode* node = (p->state == 1) ? p->then_head : p->catch_head;
    while (node) {
        tscc_microtask_enqueue(node->cb, p->value, node->ctx, node->next_promise);
        node = node->next;
    }
    /* Wake a waiting fiber */
    if (p->waiting_fiber) {
        p->waiting_fiber->resumer = tscc_current_fiber;  /* record who to return to */
        tscc_fiber_resume(p->waiting_fiber);
        p->waiting_fiber = NULL;
    }
}

void tscc_promise_resolve(MgPromise* p, void* value) {
    if (p->state != 0) return;  /* already settled */
    p->state = 1;
    p->value = value;
    tscc_fire_callbacks(p);
}

void tscc_promise_reject(MgPromise* p, void* reason) {
    if (p->state != 0) return;
    p->state = 2;
    p->value = reason;
    p->fiber_rejected = 1;
    tscc_fire_callbacks(p);
}

/* Append a then/catch node — returns a new chained promise. */
static MgThenNode* tscc_append_then_node(MgPromise* p, MgPromiseCallback cb, void* ctx,
                                         int is_catch) {
    MgThenNode* node = (MgThenNode*)calloc(1, sizeof(MgThenNode));
    node->cb  = cb;
    node->ctx = ctx;
    node->next_promise = tscc_promise_new();
    if (is_catch) {
        /* Append to catch list */
        MgThenNode** tail = &p->catch_head;
        while (*tail) tail = &(*tail)->next;
        *tail = node;
    } else {
        MgThenNode** tail = &p->then_head;
        while (*tail) tail = &(*tail)->next;
        *tail = node;
    }
    return node;
}

MgPromise* tscc_promise_then(MgPromise* p,
                              MgPromiseCallback cb, void* ctx) {
    MgThenNode* node = tscc_append_then_node(p, cb, ctx, 0);
    if (p->state == 1) {
        /* Already resolved — enqueue immediately */
        tscc_microtask_enqueue(cb, p->value, ctx, node->next_promise);
    }
    return node->next_promise;
}

MgPromise* tscc_promise_catch(MgPromise* p,
                               MgPromiseCallback cb, void* ctx) {
    MgThenNode* node = tscc_append_then_node(p, cb, ctx, 1);
    if (p->state == 2) {
        tscc_microtask_enqueue(cb, p->value, ctx, node->next_promise);
    }
    return node->next_promise;
}

/* Promise.resolve(v) — already-resolved promise */
MgPromise* tscc_promise_resolve_val(void* value) {
    MgPromise* p = tscc_promise_new();
    tscc_promise_resolve(p, value);
    return p;
}

/* Promise.reject(r) */
MgPromise* tscc_promise_reject_val(void* reason) {
    MgPromise* p = tscc_promise_new();
    tscc_promise_reject(p, reason);
    return p;
}

/* Suspend current fiber until promise settles; return resolved value.
 * If rejected, calls tscc_throw with the rejection reason. */
void* tscc_await(MgPromise* p) {
    if (p->state == 1) return p->value;
    if (p->state == 2) { tscc_throw(p->value); return NULL; }
    /* Pending — register our fiber and yield */
    MgFiber* me = tscc_current_fiber;
    if (!me) {
        fprintf(stderr, "tscc: await called outside of an async fiber\n");
        exit(1);
    }
    p->waiting_fiber = me;
    tscc_fiber_yield();
    /* Resumed after promise settled */
    if (p->state == 2) { tscc_throw(p->value); return NULL; }
    return p->value;
}

/* Promise.all — resolves when all promises resolve, rejects on first rejection. */
typedef struct { MgPromise** arr; int count; int resolved; void** values; MgPromise* out; } AllCtx;
static void tscc_all_cb(void* val, void* ctx_raw) {
    /* Simple approach: check if all are settled; if so resolve */
    AllCtx* ctx = (AllCtx*)ctx_raw;
    ctx->resolved++;
    if (ctx->resolved == ctx->count) {
        /* Collect values */
        void** vals = (void**)calloc((size_t)ctx->count, sizeof(void*));
        for (int i = 0; i < ctx->count; i++) vals[i] = ctx->arr[i]->value;
        (void)vals; /* TODO: pass as array */
        tscc_promise_resolve(ctx->out, val);
    }
}
MgPromise* tscc_promise_all(MgPromise** arr, int count) {
    MgPromise* out = tscc_promise_new();
    if (count == 0) { tscc_promise_resolve(out, NULL); return out; }
    AllCtx* ctx = (AllCtx*)calloc(1, sizeof(AllCtx));
    ctx->arr = arr; ctx->count = count; ctx->out = out;
    for (int i = 0; i < count; i++) {
        tscc_promise_then(arr[i], tscc_all_cb, ctx);
        if (arr[i]->state == 2) {
            tscc_promise_reject(out, arr[i]->value);
            return out;
        }
    }
    return out;
}

/* Promise.race */
static void tscc_race_cb(void* val, void* ctx_raw) {
    MgPromise* out = (MgPromise*)ctx_raw;
    if (out->state == 0) tscc_promise_resolve(out, val);
}
MgPromise* tscc_promise_race(MgPromise** arr, int count) {
    MgPromise* out = tscc_promise_new();
    for (int i = 0; i < count; i++) {
        if (arr[i]->state == 1) { tscc_promise_resolve(out, arr[i]->value); return out; }
        if (arr[i]->state == 2) { tscc_promise_reject(out, arr[i]->value); return out; }
        tscc_promise_then(arr[i], tscc_race_cb, out);
    }
    return out;
}

// ============================================================
// Event loop  (microtask + macrotask queues)
// ============================================================

/* --- Microtask queue (doubly-linked ring buffer would be ideal; using simple array) --- */
typedef struct {
    MgPromiseCallback cb;
    void*             value;
    void*             ctx;
    MgPromise*        chained;
} Microtask;

#define TSCC_MICROTASK_CAPACITY 65536
static Microtask tscc_microtask_queue[TSCC_MICROTASK_CAPACITY];
static int       tscc_microtask_head = 0;
static int       tscc_microtask_tail = 0;

static void tscc_microtask_enqueue(MgPromiseCallback cb, void* val, void* ctx,
                                   MgPromise* chained) {
    int next = (tscc_microtask_tail + 1) % TSCC_MICROTASK_CAPACITY;
    if (next == tscc_microtask_head) {
        fprintf(stderr, "tscc: microtask queue overflow\n"); exit(1);
    }
    tscc_microtask_queue[tscc_microtask_tail] = (Microtask){ cb, val, ctx, chained };
    tscc_microtask_tail = next;
}

static int tscc_microtask_empty(void) {
    return tscc_microtask_head == tscc_microtask_tail;
}

static void tscc_drain_microtasks(void) {
    while (!tscc_microtask_empty()) {
        Microtask t = tscc_microtask_queue[tscc_microtask_head];
        tscc_microtask_head = (tscc_microtask_head + 1) % TSCC_MICROTASK_CAPACITY;
        /* Run the callback */
        if (t.cb) {
            t.cb(t.value, t.ctx);
        }
    }
}

/* --- Macrotask queue (setTimeout / setInterval) --- */
typedef struct MacroTask {
    void            (*cb)(void*);
    void*            arg;
    long long        fire_at_ms;   /* absolute time in ms */
    struct MacroTask* next;
} MacroTask;

static MacroTask* tscc_macro_head = NULL;

static long long tscc_now_ms(void) {
    struct timespec ts;
#if defined(_WIN32) || defined(_WIN64)
    /* Use GetSystemTimeAsFileTime converted to ms */
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    long long t = ((long long)ft.dwHighDateTime << 32) | ft.dwLowDateTime;
    return t / 10000 - 11644473600000LL; /* convert to Unix ms */
#elif defined(CLOCK_REALTIME)
    clock_gettime(CLOCK_REALTIME, &ts);
    return (long long)ts.tv_sec * 1000LL + (long long)(ts.tv_nsec / 1000000LL);
#else
    return (long long)time(NULL) * 1000LL;
#endif
}

void tscc_set_timeout(void (*cb)(void*), void* arg, long long delay_ms) {
    MacroTask* t = (MacroTask*)calloc(1, sizeof(MacroTask));
    t->cb        = cb;
    t->arg       = arg;
    t->fire_at_ms = tscc_now_ms() + (delay_ms < 0 ? 0 : delay_ms);
    /* Insert in sorted order (ascending fire_at_ms) */
    MacroTask** pos = &tscc_macro_head;
    while (*pos && (*pos)->fire_at_ms <= t->fire_at_ms)
        pos = &(*pos)->next;
    t->next = *pos;
    *pos = t;
}

/* Run the event loop: drain microtasks, then fire due macrotasks, repeat. */
void tscc_event_loop_run(void) {
    while (1) {
        tscc_drain_microtasks();

        /* Check for due macrotasks */
        long long now = tscc_now_ms();
        if (!tscc_macro_head || tscc_macro_head->fire_at_ms > now) {
            /* If there's a pending macrotask in the future, sleep until it fires */
            if (tscc_macro_head) {
                long long wait_ms = tscc_macro_head->fire_at_ms - now;
                if (wait_ms > 0) {
#if defined(_WIN32) || defined(_WIN64)
                    Sleep((DWORD)wait_ms);
#else
                    struct timespec req;
                    req.tv_sec  = (time_t)(wait_ms / 1000);
                    req.tv_nsec = (long)((wait_ms % 1000) * 1000000L);
                    nanosleep(&req, NULL);
#endif
                }
                continue;
            }
            /* Nothing left — exit */
            break;
        }

        /* Pop and fire the next due macrotask */
        MacroTask* t = tscc_macro_head;
        tscc_macro_head = t->next;
        t->cb(t->arg);
        free(t);
    }
}
