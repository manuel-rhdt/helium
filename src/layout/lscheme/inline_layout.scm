
; creates a list of formatting elements
(define (do-inline-layout inline-formatting-context) 
    (process-beginning cur-elem)
    (process-content cur-elem)
    (process-end cur-elem)
)