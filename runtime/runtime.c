#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <time.h>

/* Platform-specific headers for CSPRNG (used by crypto.randomUUID()) */
#if defined(_WIN32) || defined(_WIN64)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
#elif defined(__linux__)
#  include <sys/syscall.h>
#  include <unistd.h>
#endif

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
