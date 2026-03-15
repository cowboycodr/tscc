// v0.2 Standard Library Demo

// --- Math functions ---
console.log("=== Math ===");
console.log("floor(3.7):", Math.floor(3.7));
console.log("ceil(3.2):", Math.ceil(3.2));
console.log("round(3.5):", Math.round(3.5));
console.log("abs(-42):", Math.abs(-42));
console.log("sqrt(144):", Math.sqrt(144));
console.log("pow(2, 10):", Math.pow(2, 10));
console.log("min(5, 3):", Math.min(5, 3));
console.log("max(5, 3):", Math.max(5, 3));
console.log("PI:", Math.PI);
console.log("E:", Math.E);

// --- String methods ---
console.log("\n=== String Methods ===");
let s: string = "Hello, World!";
console.log("length:", s.length);
console.log("upper:", s.toUpperCase());
console.log("lower:", s.toLowerCase());
console.log("charAt(0):", s.charAt(0));
console.log("indexOf('World'):", s.indexOf("World"));
console.log("includes('World'):", s.includes("World"));
console.log("includes('xyz'):", s.includes("xyz"));
console.log("substring(0, 5):", s.substring(0, 5));
console.log("slice(7, 12):", s.slice(7, 12));

let padded: string = "  trim me  ";
console.log("trim:", padded.trim());

// --- typeof ---
console.log("\n=== typeof ===");
let x: number = 42;
let name: string = "Mango";
let flag: boolean = true;
console.log(typeof x);
console.log(typeof name);
console.log(typeof flag);

// --- parseInt / parseFloat ---
console.log("\n=== parseInt / parseFloat ===");
console.log("parseInt('42'):", parseInt("42"));
console.log("parseFloat('3.14'):", parseFloat("3.14"));
console.log("parseInt('100abc'):", parseInt("100abc"));

// --- console.error / console.warn ---
console.error("This goes to stderr");
console.warn("This is a warning on stderr");

console.log("\nDone!");
