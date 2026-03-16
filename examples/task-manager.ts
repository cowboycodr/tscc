// A small CLI-style task manager demonstrating many TypeScript features

// ---------- Enums ----------
enum TaskStatus {
  Todo = "todo",
  InProgress = "in-progress",
  Done = "done"
}

// ---------- Interfaces ----------
interface Identifiable {
  readonly id: string
}

interface Task extends Identifiable {
  title: string
  status: TaskStatus
  createdAt: Date
}

// ---------- Utility Types ----------
type TaskUpdate = Partial<Omit<Task, "id" | "createdAt">>

// ---------- Union Types ----------
type Result<T> =
  | { success: true; data: T }
  | { success: false; error: string }

// ---------- Generic Repository ----------
class Repository<T extends Identifiable> {
  protected items: Map<string, T> = new Map()

  add(item: T): void {
    this.items.set(item.id, item)
  }

  get(id: string): T | undefined {
    return this.items.get(id)
  }

  getAll(): T[] {
    return [...this.items.values()]
  }

  update(id: string, updater: (item: T) => T): boolean {
    const item = this.items.get(id)
    if (!item) return false
    this.items.set(id, updater(item))
    return true
  }

  delete(id: string): boolean {
    return this.items.delete(id)
  }
}

// ---------- Class Inheritance ----------
class TaskService extends Repository<Task> {

  create(title: string): Task {
    const task: Task = {
      id: crypto.randomUUID(),
      title,
      status: TaskStatus.Todo,
      createdAt: new Date()
    }

    this.add(task)
    return task
  }

  updateTask(id: string, update: TaskUpdate): Result<Task> {
    const task = this.get(id)

    if (!task) {
      return { success: false, error: "Task not found" }
    }

    const updated = { ...task, ...update }

    this.items.set(id, updated)

    return { success: true, data: updated }
  }
}

// ---------- Generic Helper ----------
function printList<T>(items: T[], formatter: (item: T) => string) {
  items.forEach(i => console.log(formatter(i)))
}

// ---------- Type Guard ----------
function isSuccess<T>(result: Result<T>): result is { success: true; data: T } {
  return result.success
}

// ---------- Async Simulation ----------
async function fakeSave(): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, 300))
}

// ---------- Mapped Type Example ----------
type StatusCounts = {
  [K in TaskStatus]: number
}

function countStatuses(tasks: Task[]): StatusCounts {
  const counts: StatusCounts = {
    [TaskStatus.Todo]: 0,
    [TaskStatus.InProgress]: 0,
    [TaskStatus.Done]: 0
  }

  for (const task of tasks) {
    counts[task.status]++
  }

  return counts
}

// ---------- Main App ----------
async function main() {
  const service = new TaskService()

  const t1 = service.create("Learn TypeScript")
  const t2 = service.create("Build a small app")

  const update = service.updateTask(t1.id, { status: TaskStatus.InProgress })

  if (isSuccess(update)) {
    console.log("Updated:", update.data.title)
  } else {
    console.error(update.error)
  }

  await fakeSave()

  const tasks = service.getAll()

  console.log("\nTasks:")
  printList(tasks, t => `${t.title} [${t.status}]`)

  const stats = countStatuses(tasks)

  console.log("\nStatus Counts:")
  console.log(stats)
}

main()
