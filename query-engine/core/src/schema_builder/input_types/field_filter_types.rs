use super::*;
use datamodel_connector::ConnectorCapability;
use prisma_models::{dml::DefaultValue, PrismaValue};

/// Builds filter types for the given model field.
pub(crate) fn get_field_filter_types(
    ctx: &mut BuilderContext,
    field: &ModelField,
    include_aggregates: bool,
) -> Vec<InputType> {
    match field {
        ModelField::Relation(rf) => {
            let mut types = vec![InputType::object(full_relation_filter(ctx, rf))];
            types.extend(mto1_relation_filter_shorthand_types(ctx, rf));
            types
        }
        ModelField::Scalar(sf) if field.is_list() => vec![InputType::object(scalar_list_filter_type(ctx, sf))],
        ModelField::Scalar(sf) => {
            let mut types = vec![InputType::object(full_scalar_filter_type(
                ctx,
                &sf.type_identifier,
                sf.is_list,
                !sf.is_required,
                false,
                include_aggregates,
            ))];

            if sf.type_identifier != TypeIdentifier::Json {
                types.push(map_scalar_input_type_for_field(ctx, sf)); // Scalar equality shorthand

                if !sf.is_required {
                    types.push(InputType::null()); // Scalar null-equality shorthand
                }
            }

            types
        }
    }
}

/// Builds shorthand relation equality (`is`) filter for to-one: `where: { relation_field: { ... } }` (no `is` in between).
/// If the field is also not required, null is also added as possible type.
fn mto1_relation_filter_shorthand_types(ctx: &mut BuilderContext, rf: &RelationFieldRef) -> Vec<InputType> {
    let mut types = vec![];

    if !rf.is_list {
        let related_model = rf.related_model();
        let related_input_type = filter_objects::where_object_type(ctx, &related_model);
        types.push(InputType::object(related_input_type));

        if !rf.is_required {
            types.push(InputType::null());
        }
    }

    types
}

fn full_relation_filter(ctx: &mut BuilderContext, rf: &RelationFieldRef) -> InputObjectTypeWeakRef {
    let related_model = rf.related_model();
    let related_input_type = filter_objects::where_object_type(ctx, &related_model);
    let list = if rf.is_list { "List" } else { "" };
    let ident = Identifier::new(
        format!("{}{}RelationFilter", capitalize(&related_model.name), list),
        PRISMA_NAMESPACE,
    );

    return_cached_input!(ctx, &ident);
    let object = Arc::new(init_input_object_type(ident.clone()));
    ctx.cache_input_type(ident, object.clone());

    let fields = if rf.is_list {
        vec![
            input_field("every", InputType::object(related_input_type.clone()), None).optional(),
            input_field("some", InputType::object(related_input_type.clone()), None).optional(),
            input_field("none", InputType::object(related_input_type), None).optional(),
        ]
    } else {
        vec![
            input_field("is", InputType::object(related_input_type.clone()), None)
                .optional()
                .nullable_if(!rf.is_required),
            input_field("isNot", InputType::object(related_input_type), None)
                .optional()
                .nullable_if(!rf.is_required),
        ]
    };

    object.set_fields(fields);
    Arc::downgrade(&object)
}

fn scalar_list_filter_type(ctx: &mut BuilderContext, sf: &ScalarFieldRef) -> InputObjectTypeWeakRef {
    let ident = Identifier::new(
        scalar_filter_name(&sf.type_identifier, true, !sf.is_required, false, false),
        PRISMA_NAMESPACE,
    );
    return_cached_input!(ctx, &ident);

    let object = Arc::new(init_input_object_type(ident.clone()));
    ctx.cache_input_type(ident, object.clone());

    let mapped_type = map_scalar_input_type_for_field(ctx, sf);
    let fields = equality_filters(mapped_type, !sf.is_required).collect();
    object.set_fields(fields);

    Arc::downgrade(&object)
}

fn full_scalar_filter_type(
    ctx: &mut BuilderContext,
    typ: &TypeIdentifier,
    list: bool,
    nullable: bool,
    nested: bool,
    include_aggregates: bool,
) -> InputObjectTypeWeakRef {
    let ident = Identifier::new(
        scalar_filter_name(typ, list, nullable, nested, include_aggregates),
        PRISMA_NAMESPACE,
    );
    return_cached_input!(ctx, &ident);

    let object = Arc::new(init_input_object_type(ident.clone()));
    ctx.cache_input_type(ident, object.clone());

    let mapped_scalar_type = map_scalar_input_type(ctx, typ, list);

    let mut fields: Vec<_> = match typ {
        TypeIdentifier::String | TypeIdentifier::UUID => equality_filters(mapped_scalar_type.clone(), nullable)
            .chain(inclusion_filters(mapped_scalar_type.clone(), nullable))
            .chain(alphanumeric_filters(mapped_scalar_type.clone()))
            .chain(string_filters(mapped_scalar_type.clone()))
            .chain(query_mode_field(ctx, nested))
            .collect(),

        TypeIdentifier::Int
        | TypeIdentifier::BigInt
        | TypeIdentifier::Float
        | TypeIdentifier::DateTime
        | TypeIdentifier::Decimal => equality_filters(mapped_scalar_type.clone(), nullable)
            .chain(inclusion_filters(mapped_scalar_type.clone(), nullable))
            .chain(alphanumeric_filters(mapped_scalar_type.clone()))
            .collect(),

        TypeIdentifier::Boolean | TypeIdentifier::Json | TypeIdentifier::Xml | TypeIdentifier::Bytes => {
            equality_filters(mapped_scalar_type.clone(), nullable).collect()
        }

        TypeIdentifier::Enum(_) => equality_filters(mapped_scalar_type.clone(), nullable)
            .chain(inclusion_filters(mapped_scalar_type.clone(), nullable))
            .collect(),
    };

    // Shorthand `not equals` filter, skips the nested object filter.
    let mut not_types = vec![mapped_scalar_type.clone()];

    if typ != &TypeIdentifier::Json {
        // Full nested filter. Only available on non-JSON fields.
        not_types.push(InputType::object(full_scalar_filter_type(
            ctx,
            typ,
            list,
            nullable,
            true,
            include_aggregates,
        )));
    }

    let not_field = input_field("not", not_types, None).optional().nullable_if(nullable);
    fields.push(not_field);

    if include_aggregates {
        fields.push(aggregate_filter_field(
            ctx,
            "count",
            &TypeIdentifier::Int,
            nullable,
            list,
        ));

        if typ.is_numeric() {
            let avg_type = map_avg_type_ident(typ.clone());
            fields.push(aggregate_filter_field(ctx, "avg", &avg_type, nullable, list));
            fields.push(aggregate_filter_field(ctx, "sum", typ, nullable, list));
        }

        if !list {
            fields.push(aggregate_filter_field(ctx, "min", typ, nullable, list));
            fields.push(aggregate_filter_field(ctx, "max", typ, nullable, list));
        }
    }

    object.set_fields(fields);
    Arc::downgrade(&object)
}

fn equality_filters(mapped_type: InputType, nullable: bool) -> impl Iterator<Item = InputField> {
    vec![input_field("equals", mapped_type, None)
        .optional()
        .nullable_if(nullable)]
    .into_iter()
}

fn inclusion_filters(mapped_type: InputType, nullable: bool) -> impl Iterator<Item = InputField> {
    let typ = InputType::list(mapped_type);

    vec![
        input_field("in", typ.clone(), None).optional().nullable_if(nullable),
        input_field("notIn", typ, None).optional().nullable_if(nullable), // Kept for legacy reasons!
    ]
    .into_iter()
}

fn alphanumeric_filters(mapped_type: InputType) -> impl Iterator<Item = InputField> {
    vec![
        input_field("lt", mapped_type.clone(), None).optional(),
        input_field("lte", mapped_type.clone(), None).optional(),
        input_field("gt", mapped_type.clone(), None).optional(),
        input_field("gte", mapped_type, None).optional(),
    ]
    .into_iter()
}

fn string_filters(mapped_type: InputType) -> impl Iterator<Item = InputField> {
    vec![
        input_field("contains", mapped_type.clone(), None).optional(),
        input_field("startsWith", mapped_type.clone(), None).optional(),
        input_field("endsWith", mapped_type, None).optional(),
    ]
    .into_iter()
}

fn query_mode_field(ctx: &BuilderContext, nested: bool) -> impl Iterator<Item = InputField> {
    // Limit query mode field to the topmost filter level.
    // Only build mode field for connectors with insensitive filter support.
    let fields = if !nested && ctx.capabilities.contains(ConnectorCapability::InsensitiveFilters) {
        let enum_type = Arc::new(string_enum_type(
            "QueryMode",
            vec!["default".to_owned(), "insensitive".to_owned()],
        ));

        let field = input_field(
            "mode",
            InputType::enum_type(enum_type),
            Some(DefaultValue::Single(PrismaValue::Enum("default".to_owned()))),
        )
        .optional();

        vec![field]
    } else {
        vec![]
    };

    fields.into_iter()
}

fn scalar_filter_name(
    typ: &TypeIdentifier,
    list: bool,
    nullable: bool,
    nested: bool,
    include_aggregates: bool,
) -> String {
    let list = if list { "List" } else { "" };
    let nullable = if nullable { "Nullable" } else { "" };
    let nested = if nested { "Nested" } else { "" };
    let aggregates = if include_aggregates { "WithAggregates" } else { "" };

    match typ {
        TypeIdentifier::UUID => format!("{}Uuid{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::String => format!("{}String{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Int => format!("{}Int{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::BigInt => format!("{}BigInt{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Float => format!("{}Float{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Decimal => format!("{}Decimal{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Boolean => format!("{}Bool{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::DateTime => format!("{}DateTime{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Json => format!("{}Json{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Enum(ref e) => format!("{}Enum{}{}{}{}Filter", nested, e, nullable, list, aggregates),
        TypeIdentifier::Xml => format!("{}Xml{}{}{}Filter", nested, nullable, list, aggregates),
        TypeIdentifier::Bytes => format!("{}Bytes{}{}{}Filter", nested, nullable, list, aggregates),
    }
}

fn aggregate_filter_field(
    ctx: &mut BuilderContext,
    aggregation: &str,
    typ: &TypeIdentifier,
    nullable: bool,
    list: bool,
) -> InputField {
    let filters = full_scalar_filter_type(ctx, typ, list, nullable, true, false);
    input_field(aggregation, InputType::object(filters), None).optional()
}

fn map_avg_type_ident(typ: TypeIdentifier) -> TypeIdentifier {
    match &typ {
        TypeIdentifier::Int | TypeIdentifier::BigInt | TypeIdentifier::Float => TypeIdentifier::Float,
        _ => typ,
    }
}
