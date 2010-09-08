#include "rust_internal.h"

rust_kernel::rust_kernel(rust_srv *srv) :
    _region(&srv->local_region),
    _log(srv, NULL),
    _srv(srv),
    _interrupt_kernel_loop(FALSE),
    domains(&srv->local_region),
    message_queues(&srv->local_region) {
    // Nop.
}

rust_handle<rust_dom> *
rust_kernel::create_domain(const rust_crate *crate, const char *name) {
    rust_message_queue *message_queue =
        new (this) rust_message_queue(_srv, this);
    rust_srv *srv = _srv->clone();
    rust_dom *dom =
        new (this) rust_dom(this, message_queue, srv, crate, name);
    rust_handle<rust_dom> *handle = get_dom_handle(dom);
    message_queue->associate(handle);
    domains.append(dom);
    message_queues.append(message_queue);
    return handle;
}

void
rust_kernel::destroy_domain(rust_dom *dom) {
    log(rust_log::KERN, "deleting domain: " PTR ", index: %d, domains %d",
        dom, dom->list_index, domains.length());
    domains.remove(dom);
    dom->message_queue->disassociate();
    rust_srv *srv = dom->srv;
    delete dom;
    delete srv;
}

rust_handle<rust_dom> *
rust_kernel::get_dom_handle(rust_dom *dom) {
    rust_handle<rust_dom> *handle = NULL;
    if (_dom_handles.get(dom, &handle)) {
        return handle;
    }
    handle = new (this) rust_handle<rust_dom>(this, dom->message_queue, dom);
    _dom_handles.put(dom, handle);
    return handle;
}

rust_handle<rust_task> *
rust_kernel::get_task_handle(rust_task *task) {
    rust_handle<rust_task> *handle = NULL;
    if (_task_handles.get(task, &handle)) {
        return handle;
    }
    handle = new (this) rust_handle<rust_task>(this, task->dom->message_queue,
                                               task);
    _task_handles.put(task, handle);
    return handle;
}

rust_handle<rust_port> *
rust_kernel::get_port_handle(rust_port *port) {
    rust_handle<rust_port> *handle = NULL;
    if (_port_handles.get(port, &handle)) {
        return handle;
    }
    handle = new (this) rust_handle<rust_port>(this,
       port->task->dom->message_queue, port);
    _port_handles.put(port, handle);
    return handle;
}

void
rust_kernel::join_all_domains() {
    // TODO: Perhaps we can do this a little smarter. Just spin wait for now.
    while (domains.length() > 0) {
        sync::yield();
    }
}

void
rust_kernel::log_all_domain_state() {
    log(rust_log::KERN, "log_all_domain_state: %d domains", domains.length());
    for (uint32_t i = 0; i < domains.length(); i++) {
        domains[i]->log_state();
    }
}

/**
 * Checks for simple deadlocks.
 */
bool
rust_kernel::is_deadlocked() {
    return false;
//    _lock.lock();
//    if (domains.length() != 1) {
//        // We can only check for deadlocks when only one domain exists.
//        return false;
//    }
//
//    if (domains[0]->running_tasks.length() != 0) {
//        // We are making progress and therefore we are not deadlocked.
//        return false;
//    }
//
//    if (domains[0]->message_queue->is_empty() &&
//        domains[0]->blocked_tasks.length() > 0) {
//        // We have no messages to process, no running tasks to schedule
//        // and some blocked tasks therefore we are likely in a deadlock.
//        log_all_domain_state();
//        return true;
//    }
//
//    _lock.unlock();
//    return false;
}


void
rust_kernel::log(uint32_t type_bits, char const *fmt, ...) {
    char buf[256];
    if (_log.is_tracing(type_bits)) {
        va_list args;
        va_start(args, fmt);
        vsnprintf(buf, sizeof(buf), fmt, args);
        _log.trace_ln(NULL, type_bits, buf);
        va_end(args);
    }
}

void
rust_kernel::start_kernel_loop() {
    while (_interrupt_kernel_loop == false) {
        message_queues.global.lock();
        for (size_t i = 0; i < message_queues.length(); i++) {
            rust_message_queue *queue = message_queues[i];
            if (queue->is_associated() == false) {
                rust_message *message = NULL;
                while (queue->dequeue(&message)) {
                    message->kernel_process();
                    delete message;
                }
            }
        }
        message_queues.global.unlock();
    }
}

void
rust_kernel::run() {
    log(rust_log::KERN, "started kernel loop");
    start_kernel_loop();
    log(rust_log::KERN, "finished kernel loop");
}

rust_kernel::~rust_kernel() {
    K(_srv, domains.length() == 0,
      "Kernel has %d live domain(s), join all domains before killing "
       "the kernel.", domains.length());

    // If the kernel loop is running, interrupt it, join and exit.
    if (is_running()) {
        _interrupt_kernel_loop = true;
        join();
    }

    free_handles(_task_handles);
    free_handles(_port_handles);
    free_handles(_dom_handles);

    rust_message_queue *queue = NULL;
    while (message_queues.pop(&queue)) {
        K(_srv, queue->is_empty(), "Kernel message queue should be empty "
          "before killing the kernel.");
        delete queue;
    }
}

void *
rust_kernel::malloc(size_t size) {
    return _region->malloc(size);
}

void rust_kernel::free(void *mem) {
    _region->free(mem);
}

template<class T> void
rust_kernel::free_handles(hash_map<T*, rust_handle<T>* > &map) {
    T* key;
    rust_handle<T> *value;
    while (map.pop(&key, &value)) {
        delete value;
    }
}