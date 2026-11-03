
use super::*;

pub fn default_environment<'a>() -> Result<EnvRef<'a>, RuntimeError> {
    let mut new_map = HashMap::new();

    fn plus<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        args.iter().try_fold(Value::Number(0.0), |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a + b))
        })
    }
    new_map.insert("+".to_string(), Value::NativeFunc(plus));

    fn minus<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let first_arg = args
            .get(0)
            .ok_or(RuntimeError::Message(format!("called - without arguments")))?
            .clone();
        (&args[1..]).iter().try_fold(first_arg, |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a - b))
        })
    }
    new_map.insert("-".to_string(), Value::NativeFunc(minus));

    fn multiply<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        args.iter().try_fold(Value::Number(1.0), |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a * b))
        })
    }
    new_map.insert("*".to_string(), Value::NativeFunc(multiply));

    fn divide<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let first_arg = args
            .get(0)
            .ok_or(RuntimeError::Message("called / without arguments".to_string()))?
            .clone();
        (&args[1..]).iter().try_fold(first_arg, |a, b| {
            let a = expect_number(&a)?;
            let b = expect_number(b)?;
            Ok(Value::Number(a / b))
        })
    }
    new_map.insert("/".to_string(), Value::NativeFunc(divide));

    fn num_eq<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("= requires at least 2 arguments".to_string()));
        }
        let first = expect_number(&args[0])?;
        let mut ok = true;
        for v in &args[1..] {
            ok &= expect_number(v)? == first;
        }
        Ok(Value::Bool(ok))
    }
    new_map.insert("=".to_string(), Value::NativeFunc(num_eq));

    fn num_lt<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("< requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev < n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert("<".to_string(), Value::NativeFunc(num_lt));

    fn num_gt<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("> requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev > n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert(">".to_string(), Value::NativeFunc(num_gt));

    fn num_lte<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message("<= requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev <= n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert("<=".to_string(), Value::NativeFunc(num_lte));

    fn num_gte<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError::Message(">= requires at least 2 arguments".to_string()));
        }
        let mut prev = expect_number(&args[0])?;
        for v in &args[1..] {
            let n = expect_number(v)?;
            if !(prev >= n) { return Ok(Value::Bool(false)); }
            prev = n;
        }
        Ok(Value::Bool(true))
    }
    new_map.insert(">=".to_string(), Value::NativeFunc(num_gte));

    fn not<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("not requires exactly one argument".to_string()));
        }
        Ok(Value::Bool(!args[0].is_truthy()))
    }
    new_map.insert("not".to_string(), Value::NativeFunc(not));

    fn equal<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::Message("equal? requires exactly 2 arguments".to_string()));
        }
        Ok(Value::Bool(args[0] == args[1]))
    }
    new_map.insert("equal?".to_string(), Value::NativeFunc(equal));

    fn length<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message(
                "length expects one argument".to_string(),
            ));
        }
        match &args[0] {
            Value::List(list) => Ok(Value::Number(list.len() as f64)),
            _ => Err(RuntimeError::Message("length expects a list".to_string())),
        }
    }
    new_map.insert("length".to_string(), Value::NativeFunc(length));

    fn cons<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::Message("cons requires exactly 2 arguments".to_string()));
        }
        match &args[1] {
            Value::List(tail) => {
                let mut new_list = tail.clone();
                new_list.push_front(args[0].clone());
                Ok(Value::List(new_list))
            }
            Value::Null => Ok(Value::list(vec![args[0].clone()])),
            _ => Err(RuntimeError::Message("cons: second argument must be a list".to_string())),
        }
    }
    new_map.insert("cons".to_string(), Value::NativeFunc(cons));

    fn car<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("car requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::List(list) if !list.is_empty() => Ok(list[0].clone()),
            _ => Err(RuntimeError::Message(format!("car: expected non-empty list, got {:?}", args[0]))),
        }
    }
    new_map.insert("car".to_string(), Value::NativeFunc(car));

    fn cdr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("cdr requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::List(list) if !list.is_empty() => Ok(Value::List(list.skip(1))),
            Value::List(_) => Err(RuntimeError::Message("cdr: empty list".to_string())),
            _ => Err(RuntimeError::Message(format!("cdr: expected list, got {:?}", args[0]))),
        }
    }
    new_map.insert("cdr".to_string(), Value::NativeFunc(cdr));

    fn list<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        Ok(Value::list(args.to_vec()))
    }
    new_map.insert("list".to_string(), Value::NativeFunc(list));

    fn is_null<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("null? requires exactly 1 argument".to_string()));
        }
        let result = match &args[0] {
            Value::Null => true,
            Value::List(v) => v.is_empty(),
            _ => false,
        };
        Ok(Value::Bool(result))
    }
    new_map.insert("null?".to_string(), Value::NativeFunc(is_null));

    fn is_pair<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("pair? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(matches!(&args[0], Value::List(v) if !v.is_empty())))
    }
    new_map.insert("pair?".to_string(), Value::NativeFunc(is_pair));

    fn is_number<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("number? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_number()))
    }
    new_map.insert("number?".to_string(), Value::NativeFunc(is_number));

    fn is_string_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("string? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_string()))
    }
    new_map.insert("string?".to_string(), Value::NativeFunc(is_string_pred));

    fn is_symbol<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("symbol? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_symbol()))
    }
    new_map.insert("symbol?".to_string(), Value::NativeFunc(is_symbol));

    fn is_entity<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("entity? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(args[0].is_entity()))
    }
    new_map.insert("entity?".to_string(), Value::NativeFunc(is_entity));

    fn is_boolean<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("boolean? requires exactly 1 argument".to_string()));
        }
        Ok(Value::Bool(matches!(args[0], Value::Bool(_))))
    }
    new_map.insert("boolean?".to_string(), Value::NativeFunc(is_boolean));

    fn display<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("display requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::String(s) => print!("{}", s),
            other => print!("{:?}", other),
        }
        Ok(Value::Null)
    }
    new_map.insert("display".to_string(), Value::NativeFunc(display));

    fn newline<'a>(_args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        println!();
        Ok(Value::Null)
    }
    new_map.insert("newline".to_string(), Value::NativeFunc(newline));

    fn abs<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("abs requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.abs()))
    }
    new_map.insert("abs".to_string(), Value::NativeFunc(abs));

    fn floor_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("floor requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.floor()))
    }
    new_map.insert("floor".to_string(), Value::NativeFunc(floor_fn));

    fn ceiling_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("ceiling requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.ceil()))
    }
    new_map.insert("ceiling".to_string(), Value::NativeFunc(ceiling_fn));

    fn sqrt_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("sqrt requires exactly 1 argument".to_string()));
        }
        Ok(Value::Number(expect_number(&args[0])?.sqrt()))
    }
    new_map.insert("sqrt".to_string(), Value::NativeFunc(sqrt_fn));

    fn append<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let mut result = List::new();
        for arg in args {
            match arg {
                Value::List(items) => result.append(items.clone()),
                Value::Null => {}
                _ => return Err(RuntimeError::Message(format!("append: expected list, got {:?}", arg))),
            }
        }
        Ok(Value::List(result))
    }
    new_map.insert("append".to_string(), Value::NativeFunc(append));

    fn reverse<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("reverse requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::List(list) => {
                Ok(Value::list(list.iter().rev().cloned().collect::<Vec<_>>()))
            }
            _ => Err(RuntimeError::Message("reverse: expected list".to_string())),
        }
    }
    new_map.insert("reverse".to_string(), Value::NativeFunc(reverse));

    fn number_to_string<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("number->string requires exactly 1 argument".to_string()));
        }
        let n = expect_number(&args[0])?;
        Ok(Value::String(if n == n.floor() { format!("{}", n as i64) } else { format!("{}", n) }))
    }
    new_map.insert("number->string".to_string(), Value::NativeFunc(number_to_string));

    fn string_to_number<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("string->number requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::String(s) => match s.parse::<f64>() {
                Ok(n) => Ok(Value::Number(n)),
                Err(_) => Ok(Value::Bool(false)),
            },
            _ => Err(RuntimeError::Message("string->number: expected string".to_string())),
        }
    }
    new_map.insert("string->number".to_string(), Value::NativeFunc(string_to_number));

    fn string_append<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        let mut result = String::new();
        for arg in args {
            match arg {
                Value::String(s) => result.push_str(s),
                _ => return Err(RuntimeError::Message(format!("string-append: expected string, got {:?}", arg))),
            }
        }
        Ok(Value::String(result))
    }
    new_map.insert("string-append".to_string(), Value::NativeFunc(string_append));

    fn symbol_to_string<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("symbol->string requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::Symbol(s) => Ok(Value::String(s.as_str().to_string())),
            _ => Err(RuntimeError::Message("symbol->string: expected symbol".to_string())),
        }
    }
    new_map.insert("symbol->string".to_string(), Value::NativeFunc(symbol_to_string));

    fn string_to_symbol<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::Message("string->symbol requires exactly 1 argument".to_string()));
        }
        match &args[0] {
            Value::String(s) => Ok(Value::Symbol(Symbol::intern(s))),
            _ => Err(RuntimeError::Message("string->symbol: expected string".to_string())),
        }
    }
    new_map.insert("string->symbol".to_string(), Value::NativeFunc(string_to_symbol));

    fn cadr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cadr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 2 => Ok(v[1].clone()),
            _ => Err(RuntimeError::Message(format!("cadr: expected list with >=2 elements, got {:?}", args[0]))),
        }
    }
    new_map.insert("cadr".to_string(), Value::NativeFunc(cadr));

    fn caddr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("caddr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 3 => Ok(v[2].clone()),
            _ => Err(RuntimeError::Message(format!("caddr: expected list with >=3 elements, got {:?}", args[0]))),
        }
    }
    new_map.insert("caddr".to_string(), Value::NativeFunc(caddr));

    fn cadddr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cadddr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 4 => Ok(v[3].clone()),
            _ => Err(RuntimeError::Message(format!("cadddr: expected list with >=4 elements, got {:?}", args[0]))),
        }
    }
    new_map.insert("cadddr".to_string(), Value::NativeFunc(cadddr));

    fn cddr<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cddr requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if v.len() >= 2 => Ok(Value::List(v.skip(2))),
            Value::List(_) => Err(RuntimeError::Message("cddr: list too short".to_string())),
            _ => Err(RuntimeError::Message(format!("cddr: expected list, got {:?}", args[0]))),
        }
    }
    new_map.insert("cddr".to_string(), Value::NativeFunc(cddr));

    fn caar<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("caar requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if !v.is_empty() => match &v[0] {
                Value::List(inner) if !inner.is_empty() => Ok(inner[0].clone()),
                _ => Err(RuntimeError::Message("caar: car is not a non-empty list".to_string())),
            },
            _ => Err(RuntimeError::Message(format!("caar: expected non-empty list, got {:?}", args[0]))),
        }
    }
    new_map.insert("caar".to_string(), Value::NativeFunc(caar));

    fn cdar<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("cdar requires 1 argument".to_string())); }
        match &args[0] {
            Value::List(v) if !v.is_empty() => match &v[0] {
                Value::List(inner) if !inner.is_empty() => Ok(Value::List(inner.skip(1))),
                Value::List(_) => Err(RuntimeError::Message("cdar: car is empty list".to_string())),
                _ => Err(RuntimeError::Message("cdar: car is not a list".to_string())),
            },
            _ => Err(RuntimeError::Message(format!("cdar: expected non-empty list, got {:?}", args[0]))),
        }
    }
    new_map.insert("cdar".to_string(), Value::NativeFunc(cdar));

    fn eq_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 { return Err(RuntimeError::Message("eq? requires 2 arguments".to_string())); }
        let result = match (&args[0], &args[1]) {
            (Value::Symbol(a), Value::Symbol(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::List(a), Value::List(b)) => a.is_empty() && b.is_empty(),
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Entity(a), Value::Entity(b)) => a == b,
            _ => false,
        };
        Ok(Value::Bool(result))
    }
    new_map.insert("eq?".to_string(), Value::NativeFunc(eq_pred));

    fn min_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.is_empty() { return Err(RuntimeError::Message("min requires at least 1 argument".to_string())); }
        let mut m = expect_number(&args[0])?;
        for v in &args[1..] { let n = expect_number(v)?; if n < m { m = n; } }
        Ok(Value::Number(m))
    }
    new_map.insert("min".to_string(), Value::NativeFunc(min_fn));

    fn max_fn<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.is_empty() { return Err(RuntimeError::Message("max requires at least 1 argument".to_string())); }
        let mut m = expect_number(&args[0])?;
        for v in &args[1..] { let n = expect_number(v)?; if n > m { m = n; } }
        Ok(Value::Number(m))
    }
    new_map.insert("max".to_string(), Value::NativeFunc(max_fn));

    fn zero_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("zero? requires 1 argument".to_string())); }
        Ok(Value::Bool(expect_number(&args[0])? == 0.0))
    }
    new_map.insert("zero?".to_string(), Value::NativeFunc(zero_pred));

    fn positive_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("positive? requires 1 argument".to_string())); }
        Ok(Value::Bool(expect_number(&args[0])? > 0.0))
    }
    new_map.insert("positive?".to_string(), Value::NativeFunc(positive_pred));

    fn negative_pred<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 1 { return Err(RuntimeError::Message("negative? requires 1 argument".to_string())); }
        Ok(Value::Bool(expect_number(&args[0])? < 0.0))
    }
    new_map.insert("negative?".to_string(), Value::NativeFunc(negative_pred));

    fn list_ref<'a>(args: &[Value<'a>]) -> Result<Value<'a>, RuntimeError> {
        if args.len() != 2 { return Err(RuntimeError::Message("list-ref requires 2 arguments".to_string())); }
        let idx = expect_number(&args[1])? as usize;
        match &args[0] {
            Value::List(v) => v.get(idx).cloned().ok_or_else(|| RuntimeError::Message(
                format!("list-ref: index {} out of bounds (length {})", idx, v.len())
            )),
            _ => Err(RuntimeError::Message(format!("list-ref: expected list, got {:?}", args[0]))),
        }
    }
    new_map.insert("list-ref".to_string(), Value::NativeFunc(list_ref));

    let env = Rc::new(RefCell::new(Environment {
        bindings: new_map
            .into_iter()
            .map(|(name, value)| (Symbol::intern(&name), value))
            .collect(),
    }));

    // Bootstrap higher-order functions defined in Scheme (requires working recursion)
    let bootstrap = r#"
(define (for-each f lst)
  (when (pair? lst)
    (f (car lst))
    (for-each f (cdr lst))))

(define (map f lst)
  (if (null? lst)
    '()
    (cons (f (car lst)) (map f (cdr lst)))))

(define (filter f lst)
  (cond
    ((null? lst) '())
    ((f (car lst)) (cons (car lst) (filter f (cdr lst))))
    (else (filter f (cdr lst)))))

(define (assoc key lst)
  (cond
    ((null? lst) #f)
    ((equal? (caar lst) key) (car lst))
    (else (assoc key (cdr lst)))))

(define (member x lst)
  (cond
    ((null? lst) #f)
    ((equal? x (car lst)) lst)
    (else (member x (cdr lst)))))
"#;
    for form in parse_lscheme(bootstrap)? {
        eval(form, Rc::clone(&env))?;
    }

    Ok(env)
}

