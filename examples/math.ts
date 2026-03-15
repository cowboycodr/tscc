// A module that exports math utilities

export function square(x: number): number {
    return x * x;
}

export function cube(x: number): number {
    return x * x * x;
}

export function clamp(val: number, min: number, max: number): number {
    if (val < min) {
        return min;
    }
    if (val > max) {
        return max;
    }
    return val;
}
