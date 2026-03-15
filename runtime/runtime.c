#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <time.h>

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
